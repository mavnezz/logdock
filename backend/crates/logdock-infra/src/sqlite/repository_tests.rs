//! The repositories against a real database in a temporary directory.

use logdock_identity::application::ports::{
    AccountRepository, ApiTokenRepository, AuditReader, SessionRepository,
};
use logdock_identity::domain::{
    AccountId, DisplayName, Email, Locale, NewAccount, NewApiToken, NewSession, PasswordHash, Role,
    TokenHash, TokenName, TokenScope,
};
use logdock_kernel::{AuditEntry, AuditLog, Day, Outcome, ProjectId, RequestContext, Secret, Timestamp};
use logdock_projects::application::ports::{IngestCounter, KeyRepository, KeyTokens, ProjectRepository};
use logdock_projects::domain::{
    EnvironmentName, KeyLabel, NotifyAddress, ProjectName, ProjectSettings, Rejection, Tally,
};

use super::*;

pub(crate) async fn database() -> (Database, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (db, _) = Database::open(&dir.path().join("logdock.db")).await.unwrap();
    (db, dir)
}

fn new_account(email: &str, at: i64) -> NewAccount {
    NewAccount {
        email: Email::parse(email).unwrap(),
        name: DisplayName::parse("Someone").unwrap(),
        role: Role::Admin,
        locale: Locale::De,
        password_hash: PasswordHash::new("hash".into()),
        created_at: Timestamp::from_unix_seconds(at),
    }
}

#[tokio::test]
async fn stores_accounts_and_their_sessions() {
    let (db, _dir) = database().await;
    let accounts = SqliteAccounts::new(db.clone());
    let first = accounts.create_first(new_account("ada@example.com", 1)).await.unwrap().unwrap();
    assert!(accounts.create_first(new_account("bob@example.com", 2)).await.unwrap().is_none());
    assert!(accounts.create(new_account("ada@example.com", 3)).await.unwrap().is_none(), "taken");
    let bob = accounts.create(new_account("bob@example.com", 3)).await.unwrap().unwrap();
    assert_eq!(accounts.count().await.unwrap(), 2);
    assert_eq!(first.locale, Locale::De);

    let mut changed = bob.clone();
    changed.role = Role::Member;
    changed.disabled = true;
    accounts.update(&changed).await.unwrap();
    accounts
        .update_password(bob.id, &PasswordHash::new("new".into()), Timestamp::from_unix_seconds(9))
        .await
        .unwrap();
    let stored = accounts.find_by_email(&Email::parse("BOB@example.com").unwrap()).await.unwrap().unwrap();
    assert_eq!((stored.role, stored.disabled, stored.password_hash.as_str()), (Role::Member, true, "new"));
    assert_eq!(accounts.list().await.unwrap().len(), 2);

    let sessions = SqliteSessions::new(db.clone());
    let new = |token: &str, at: i64| NewSession {
        account_id: bob.id,
        token_hash: TokenHash::new(token.into()),
        csrf_token: Secret::new("csrf"),
        created_at: Timestamp::from_unix_seconds(at),
        client_ip: Some([10, 0, 0, 1].into()),
        user_agent: Some("test".into()),
    };
    let one = sessions.create(new("t1", 10)).await.unwrap();
    let two = sessions.create(new("t2", 20)).await.unwrap();
    let found = sessions.find_by_token(&TokenHash::new("t1".into())).await.unwrap().unwrap();
    assert_eq!(
        (found.id, found.csrf_token.expose(), found.client_ip),
        (one.id, "csrf", Some([10, 0, 0, 1].into()))
    );
    sessions.touch(one.id, Timestamp::from_unix_seconds(30)).await.unwrap();
    assert_eq!(
        sessions
            .delete_expired(Timestamp::from_unix_seconds(25), Timestamp::from_unix_seconds(0))
            .await
            .unwrap(),
        1
    );
    assert!(sessions.find_by_token(&TokenHash::new("t2".into())).await.unwrap().is_none());
    sessions.create(new("t3", 40)).await.unwrap();
    sessions.delete_for_account(bob.id, Some(one.id)).await.unwrap();
    assert!(sessions.find_by_token(&TokenHash::new("t1".into())).await.unwrap().is_some());
    assert!(sessions.find_by_token(&TokenHash::new("t3".into())).await.unwrap().is_none());
    sessions.delete(two.id).await.unwrap();

    accounts.delete(bob.id).await.unwrap();
    assert!(
        sessions.find_by_token(&TokenHash::new("t1".into())).await.unwrap().is_none(),
        "sessions went with it"
    );
    assert!(accounts.find_by_id(AccountId::new(bob.id.get())).await.unwrap().is_none());
}

#[tokio::test]
async fn keeps_the_audit_log() {
    let (db, _dir) = database().await;
    let audit = SqliteAudit::new(db);
    let request = RequestContext {
        client_ip: Some([10, 0, 0, 1].into()),
        user_agent: Some("ua".into()),
        ..Default::default()
    };
    for i in 0..3 {
        let entry = AuditEntry::new(Timestamp::from_unix_seconds(i), "login", Outcome::Failure, &request)
            .actor("eve")
            .reason("x");
        audit.record(entry).await.unwrap();
    }
    let entries = audit.list(None, 2).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].1.at, Timestamp::from_unix_seconds(2));
    assert_eq!(entries[0].1.client_ip, request.client_ip);
    assert_eq!(entries[0].1.outcome, Outcome::Failure);
    let older = audit.list(Some(entries[1].0), 10).await.unwrap();
    assert_eq!(older.len(), 1);
}

#[tokio::test]
async fn stores_projects_stages_keys_and_counts() {
    let (db, _dir) = database().await;
    let projects = SqliteProjects::new(db.clone());
    let name = ProjectName::parse("orders").unwrap();
    let envs = [EnvironmentName::parse("production").unwrap(), EnvironmentName::parse("staging").unwrap()];
    let at = Timestamp::from_unix_seconds(100);
    let project = projects.create(&name, &envs, at).await.unwrap().unwrap();
    assert_eq!(project.environments.len(), 2);
    assert!(
        projects.create(&ProjectName::parse("ORDERS").unwrap(), &[], at).await.unwrap().is_none(),
        "names ignore case"
    );
    let billing = projects.create(&ProjectName::parse("billing").unwrap(), &[], at).await.unwrap().unwrap();
    assert!(!projects.rename(billing.id, &name).await.unwrap());
    assert!(projects.rename(billing.id, &ProjectName::parse("invoices").unwrap()).await.unwrap());
    let names: Vec<String> = projects.list().await.unwrap().iter().map(|p| p.name.to_string()).collect();
    assert_eq!(names, ["invoices", "orders"]);

    let mut settings = ProjectSettings::default();
    settings.notify.push(NotifyAddress::parse("ops@example.com").unwrap());
    settings.notify_regressions = false;
    settings.redaction.allowed_keys = Some(vec!["order_id".into()]);
    settings.redaction.scrub_emails = true;
    let stage = project.environments[0].id;
    let short = logdock_kernel::retention::StageRetention {
        logs_days: Some(3),
        traces_days: Some(1),
        ..Default::default()
    };
    settings.retention = vec![(stage, short)];
    projects.update_settings(project.id, &settings).await.unwrap();
    assert_eq!(projects.find(project.id).await.unwrap().unwrap().settings, settings);
    let stored: String = sqlx::query_scalar("SELECT settings FROM projects WHERE id = ?")
        .bind(project.id.get())
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(super::projects::stage_retention(&stored), [(stage.get(), short)], "as the segments read it");
    assert!(super::projects::stage_retention("not json").is_empty());

    let keys = SqliteKeys::new(db.clone());
    let tokens = RandomKeys;
    let dev = EnvironmentName::parse("development").unwrap();
    let added = projects.add_environment(project.id, &dev, at).await.unwrap().unwrap();
    assert!(projects.add_environment(project.id, &dev, at).await.unwrap().is_none());
    let of_dev = tokens.hash("of-dev");
    keys.create(project.id, Some(added.id), None, "of-dev", &of_dev, at).await.unwrap();
    let whole = tokens.hash("whole");
    let label = KeyLabel::parse("laravel").unwrap();
    let of_project = keys.create(project.id, None, label.as_ref(), "whole", &whole, at).await.unwrap();
    projects.remove_environment(project.id, added.id).await.unwrap();
    assert!(keys.find_by_hash(&of_dev).await.unwrap().is_none(), "a stage's keys go with it");
    let kept = keys.find_by_hash(&whole).await.unwrap().unwrap();
    assert_eq!((kept.environment, &kept.label), (None, &label), "a key of the project stays");
    assert_eq!(kept, of_project);

    let secret = tokens.new_key();
    assert_eq!(secret.expose().len(), 32);
    let hash = tokens.hash(secret.expose());
    let key = keys
        .create(project.id, Some(project.environments[0].id), None, &secret.expose()[..8], &hash, at)
        .await
        .unwrap();
    assert_eq!(
        keys.find_by_hash(&hash).await.unwrap().unwrap().environment,
        Some(project.environments[0].id)
    );
    keys.touch(key.id, Timestamp::from_unix_seconds(200)).await.unwrap();
    keys.revoke(key.id, Timestamp::from_unix_seconds(300)).await.unwrap();
    let listed = keys.list(project.id).await.unwrap();
    assert_eq!(listed[0].last_used_at, Some(Timestamp::from_unix_seconds(200)));
    assert_eq!(listed[0].revoked_at, Some(Timestamp::from_unix_seconds(300)));

    let counter = SqliteIngestCounter::new(db.clone());
    let day = Timestamp::from_unix_seconds(86_400 * 20_000);
    counter.record(project.id, Some(key.id), &Tally::accepted(3), day).await;
    let mut refused = Tally::rejected(Rejection::Size, 2, "too large");
    refused.accepted = 1;
    counter.record(project.id, Some(key.id), &refused, day).await;
    counter.flush().await.unwrap();
    counter.record(project.id, None, &Tally::rejected(Rejection::Auth, 1, "unknown key"), day).await;
    counter.record(ProjectId::new(9999), None, &Tally::accepted(1), day).await;
    let counts = counter.counts(project.id, Day::from_days(19_999)).await.unwrap();
    assert_eq!(counts.len(), 2);
    let with_key = counts.iter().find(|c| c.key == Some(key.id)).unwrap();
    assert_eq!((with_key.accepted, with_key.rejected.clone()), (4, vec![(Rejection::Size, 2)]));
    assert_eq!(
        with_key.last_rejection.as_ref().map(|(r, d, _)| (*r, d.as_str())),
        Some((Rejection::Size, "too large"))
    );
    let without = counts.iter().find(|c| c.key.is_none()).unwrap();
    assert_eq!(without.rejected, [(Rejection::Auth, 1)]);
    assert!(counter.counts(project.id, Day::from_days(20_001)).await.unwrap().is_empty());

    projects.delete(project.id).await.unwrap();
    assert!(projects.find(project.id).await.unwrap().is_none());
    assert!(keys.find_by_hash(&hash).await.unwrap().is_none(), "keys went with it");
}

#[tokio::test]
async fn stores_api_tokens_with_their_account() {
    let (db, _dir) = database().await;
    let accounts = SqliteAccounts::new(db.clone());
    let ada = accounts.create(new_account("ada@example.com", 1)).await.unwrap().unwrap();
    let tokens = SqliteApiTokens::new(db.clone());
    let new = |name: &str, scope| NewApiToken {
        account_id: ada.id,
        name: TokenName::parse(name).unwrap(),
        scope,
        prefix: format!("ldt_{name}"),
        hash: TokenHash::new(format!("hash-{name}")),
        created_at: Timestamp::from_unix_seconds(10),
    };
    let read = tokens.create(new("read", TokenScope::Read)).await.unwrap();
    let triage = tokens.create(new("triage", TokenScope::Triage)).await.unwrap();
    assert_eq!(
        tokens.find_by_hash(&TokenHash::new("hash-triage".into())).await.unwrap(),
        Some(triage.clone())
    );
    assert!(tokens.find_by_hash(&TokenHash::new("nope".into())).await.unwrap().is_none());
    tokens.touch(read.id, Timestamp::from_unix_seconds(20)).await.unwrap();
    tokens.revoke(read.id, Timestamp::from_unix_seconds(30)).await.unwrap();
    tokens.revoke(read.id, Timestamp::from_unix_seconds(40)).await.unwrap();
    let listed = tokens.list(ada.id).await.unwrap();
    assert_eq!(listed.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["triage", "read"]);
    assert_eq!(listed[1].scope, TokenScope::Read);
    assert_eq!(listed[1].last_used_at, Some(Timestamp::from_unix_seconds(20)));
    assert_eq!(listed[1].revoked_at, Some(Timestamp::from_unix_seconds(30)), "revoked once");

    sqlx::query("UPDATE api_tokens SET scope = 'root' WHERE id = ?")
        .bind(triage.id.get())
        .execute(db.pool())
        .await
        .unwrap();
    assert!(tokens.list(ada.id).await.is_err(), "an unknown scope is an error, not a guess");
    accounts.delete(ada.id).await.unwrap();
    assert!(tokens.list(ada.id).await.unwrap().is_empty(), "tokens go with their account");
}
