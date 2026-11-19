use std::sync::atomic::Ordering;
use std::time::Duration;

use logdock_kernel::{Actor, ProjectId, RequestContext};

use crate::application::{MAX_STAGES, ProjectChanges, ProjectError, StageFor};
use crate::domain::{NotifyAddress, ProjectSettings, Rejection, Tally};
use crate::testing::TestProjects;

fn actor() -> Actor {
    Actor { account_id: 1, name: "ada@example.com".into() }
}

fn request() -> RequestContext {
    RequestContext::default()
}

#[tokio::test]
async fn creates_projects_with_their_stages() {
    let t = TestProjects::new();
    let envs = ["Production".to_owned(), "staging".to_owned(), "production".to_owned()];
    let project = t.use_cases.create.execute(&actor(), "orders-backend", &envs, &request()).await.unwrap();
    let names: Vec<&str> = project.environments.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["production", "staging"]);

    let again = t.use_cases.create.execute(&actor(), "orders-backend", &[], &request()).await;
    assert!(matches!(again, Err(ProjectError::NameTaken)));
    let bad = t.use_cases.create.execute(&actor(), "x", &["not ok".into()], &request()).await;
    assert!(matches!(bad, Err(ProjectError::Invalid(_))));

    let env =
        t.use_cases.add_environment.execute(&actor(), project.id, "development", &request()).await.unwrap();
    let taken = t.use_cases.add_environment.execute(&actor(), project.id, "staging", &request()).await;
    assert!(matches!(taken, Err(ProjectError::EnvironmentTaken)));
    t.use_cases.remove_environment.execute(&actor(), project.id, env.id, &request()).await.unwrap();
    assert_eq!(t.use_cases.get.execute(project.id).await.unwrap().environments.len(), 2);
    assert_eq!(t.use_cases.list.execute().await.unwrap().len(), 1);
    let audit: Vec<String> = t.audit.0.lock().unwrap().iter().map(|e| e.action.clone()).collect();
    assert_eq!(audit, ["project.create", "environment.add", "environment.remove"]);
}

#[tokio::test]
async fn keys_let_in_their_stage_until_revoked() {
    let t = TestProjects::new();
    let project =
        t.use_cases.create.execute(&actor(), "orders", &["production".into()], &request()).await.unwrap();
    let production = project.environments[0].id;
    let created = t
        .use_cases
        .create_key
        .execute(&actor(), project.id, Some(production), " web ", &request())
        .await
        .unwrap();
    assert_eq!(created.key.prefix.len(), 8);
    assert!(created.secret.expose().starts_with(&created.key.prefix));
    assert_eq!(created.key.label.as_ref().map(|l| l.as_str()), Some("web"));

    let auth = &t.use_cases.authenticate_key;
    let scope = auth.execute(Some(project.id), created.secret.expose()).await.unwrap().unwrap();
    let stage = scope.stage.clone().unwrap();
    assert_eq!((stage.scope.environment, stage.name.as_str()), (production, "production"));
    assert!(auth.execute(None, created.secret.expose()).await.unwrap().is_some(), "NDJSON names no project");
    assert!(auth.execute(Some(ProjectId::new(999)), created.secret.expose()).await.unwrap().is_none());
    assert!(auth.execute(Some(project.id), "unknown").await.unwrap().is_none());
    assert!(auth.execute(Some(project.id), "").await.unwrap().is_none());

    // Cached: the last use is written once a minute at most.
    let touches = t.keys.touches.load(Ordering::SeqCst);
    auth.execute(Some(project.id), created.secret.expose()).await.unwrap();
    assert_eq!(t.keys.touches.load(Ordering::SeqCst), touches);
    t.clock.advance(Duration::from_secs(61));
    auth.execute(Some(project.id), created.secret.expose()).await.unwrap();
    assert_eq!(t.keys.touches.load(Ordering::SeqCst), touches + 1);

    let in_use = t.use_cases.remove_environment.execute(&actor(), project.id, production, &request()).await;
    assert!(matches!(in_use, Err(ProjectError::EnvironmentInUse)));
    t.use_cases.revoke_key.execute(&actor(), project.id, created.key.id, &request()).await.unwrap();
    assert!(
        auth.execute(Some(project.id), created.secret.expose()).await.unwrap().is_none(),
        "revoked at once"
    );
    let keys = t.use_cases.list_keys.execute(project.id).await.unwrap();
    assert!(keys[0].revoked_at.is_some());
    t.use_cases.remove_environment.execute(&actor(), project.id, production, &request()).await.unwrap();
}

#[tokio::test]
async fn keys_need_a_stage_of_their_project() {
    let t = TestProjects::new();
    let a = t.use_cases.create.execute(&actor(), "a", &["production".into()], &request()).await.unwrap();
    let b = t.use_cases.create.execute(&actor(), "b", &["production".into()], &request()).await.unwrap();
    let foreign =
        t.use_cases.create_key.execute(&actor(), a.id, Some(b.environments[0].id), "", &request()).await;
    assert!(matches!(foreign, Err(ProjectError::EnvironmentNotFound)));
    let missing = t.use_cases.create_key.execute(&actor(), ProjectId::new(77), None, "", &request()).await;
    assert!(matches!(missing, Err(ProjectError::NotFound)));
}

#[tokio::test]
async fn a_key_of_one_stage_refuses_data_of_another() {
    let t = TestProjects::new();
    let project =
        t.use_cases.create.execute(&actor(), "orders", &["production".into()], &request()).await.unwrap();
    let created = t
        .use_cases
        .create_key
        .execute(&actor(), project.id, Some(project.environments[0].id), "", &request())
        .await
        .unwrap();
    let scope = t.use_cases.authenticate_key.execute(None, created.secret.expose()).await.unwrap().unwrap();
    let resolve = |claimed| t.use_cases.resolve_stage.execute(&scope, claimed);
    for claimed in [None, Some("production"), Some(" Production ")] {
        assert_eq!(
            resolve(claimed).await.unwrap(),
            StageFor::Stage(scope.stage.clone().unwrap()),
            "{claimed:?}"
        );
    }
    let StageFor::Refused(why) = resolve(Some("staging")).await.unwrap() else { panic!("taken") };
    assert_eq!(why, r#"data for stage "staging" sent with a key of "production""#);
    assert_eq!(t.use_cases.get.execute(project.id).await.unwrap().environments.len(), 1, "no stage added");
}

#[tokio::test]
async fn a_key_of_the_project_takes_the_stages_its_data_names() {
    let t = TestProjects::new();
    let project =
        t.use_cases.create.execute(&actor(), "orders", &["production".into()], &request()).await.unwrap();
    let created = t.use_cases.create_key.execute(&actor(), project.id, None, "", &request()).await.unwrap();
    assert_eq!(created.key.environment, None);
    let auth = &t.use_cases.authenticate_key;
    let scope = auth.execute(Some(project.id), created.secret.expose()).await.unwrap().unwrap();
    assert_eq!(scope.stage, None);
    let resolve = |claimed| t.use_cases.resolve_stage.execute(&scope, claimed);
    let stage = |answer: StageFor| match answer {
        StageFor::Stage(stage) => stage,
        StageFor::Refused(why) => panic!("refused: {why}"),
    };

    // Data that names no stage goes to production, as Sentry's SDKs default.
    let production = stage(resolve(None).await.unwrap());
    assert_eq!(production.scope.environment, project.environments[0].id);

    // A stage named for the first time is added, once.
    let qa = stage(resolve(Some("QA")).await.unwrap());
    assert_eq!((qa.name.as_str(), qa.scope.project), ("qa", project.id));
    assert_eq!(stage(resolve(Some("qa")).await.unwrap()), qa, "found again, from the stale scope");
    let stages = t.use_cases.get.execute(project.id).await.unwrap().environments;
    assert_eq!(stages.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["production", "qa"]);
    let fresh = auth.execute(Some(project.id), created.secret.expose()).await.unwrap().unwrap();
    assert_eq!(fresh.stages.len(), 2, "adding a stage renews the cached scopes");

    let StageFor::Refused(why) = resolve(Some("feature/login")).await.unwrap() else { panic!("taken") };
    assert!(why.starts_with(r#""feature/login" is no stage name"#), "{why}");

    // A stage-less key does not keep a stage from being removed.
    t.use_cases
        .remove_environment
        .execute(&actor(), project.id, qa.scope.environment, &request())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_project_gets_a_limited_number_of_stages_from_its_data() {
    let t = TestProjects::new();
    let project = t.use_cases.create.execute(&actor(), "orders", &[], &request()).await.unwrap();
    let created = t.use_cases.create_key.execute(&actor(), project.id, None, "", &request()).await.unwrap();
    let scope = t.use_cases.authenticate_key.execute(None, created.secret.expose()).await.unwrap().unwrap();
    for n in 0..MAX_STAGES {
        let name = format!("stage-{n}");
        let answer = t.use_cases.resolve_stage.execute(&scope, Some(&name)).await.unwrap();
        assert!(matches!(answer, StageFor::Stage(_)), "{name}");
    }
    let answer = t.use_cases.resolve_stage.execute(&scope, Some("one-more")).await.unwrap();
    assert!(matches!(answer, StageFor::Refused(why) if why.contains("has 50 stages")));
    let known = t.use_cases.resolve_stage.execute(&scope, Some("stage-7")).await.unwrap();
    assert!(matches!(known, StageFor::Stage(_)), "known stages still take data");
}

#[tokio::test]
async fn renames_changes_settings_and_deletes() {
    let t = TestProjects::new();
    let project =
        t.use_cases.create.execute(&actor(), "orders", &["production".into()], &request()).await.unwrap();
    t.use_cases.create.execute(&actor(), "billing", &[], &request()).await.unwrap();
    let key = t
        .use_cases
        .create_key
        .execute(&actor(), project.id, Some(project.environments[0].id), "", &request())
        .await
        .unwrap();

    let taken = ProjectChanges { name: Some("billing".into()), ..Default::default() };
    let result = t.use_cases.update.execute(&actor(), project.id, taken, &request()).await;
    assert!(matches!(result, Err(ProjectError::NameTaken)));

    let mut settings = ProjectSettings::default();
    settings.notify.push(NotifyAddress::parse("ops@example.com").unwrap());
    settings.redaction.extra_keys.push("customer_pin".into());
    let changes = ProjectChanges { name: Some("orders-api".into()), settings: Some(settings.clone()) };
    let changed = t.use_cases.update.execute(&actor(), project.id, changes, &request()).await.unwrap();
    assert_eq!(changed.name.as_str(), "orders-api");
    let scope = t.use_cases.authenticate_key.execute(None, key.secret.expose()).await.unwrap().unwrap();
    assert_eq!(scope.settings, settings, "the key's scope carries the new settings at once");

    t.use_cases.delete.execute(&actor(), project.id, &request()).await.unwrap();
    assert!(matches!(t.use_cases.get.execute(project.id).await, Err(ProjectError::NotFound)));
    assert!(t.use_cases.authenticate_key.execute(None, key.secret.expose()).await.unwrap().is_none());
}

#[tokio::test]
async fn counts_what_ingest_did() {
    let t = TestProjects::new();
    let project = t.use_cases.create.execute(&actor(), "orders", &[], &request()).await.unwrap();
    t.use_cases.record_ingest.execute(project.id, None, &Tally::accepted(5)).await;
    t.use_cases.record_ingest.execute(project.id, None, &Tally::default()).await;
    t.use_cases.record_ingest.refused(project.id, Rejection::Auth, "unknown key").await;
    assert_eq!(t.counter.0.lock().unwrap().len(), 2, "empty tallies are not recorded");
    let counts = t.use_cases.ingest_counts.execute(project.id, 7).await.unwrap();
    assert_eq!(counts[0].accepted, 5);
    assert!(matches!(
        t.use_cases.ingest_counts.execute(ProjectId::new(9), 7).await,
        Err(ProjectError::NotFound)
    ));
}

#[tokio::test]
async fn sends_a_test_mail_to_the_projects_addresses() {
    let t = TestProjects::new();
    let project = t.use_cases.create.execute(&actor(), "orders", &[], &request()).await.unwrap();
    assert!(matches!(t.use_cases.test_mail.execute(project.id).await, Err(ProjectError::NoRecipients)));
    let mut settings = ProjectSettings::default();
    settings.notify.push(NotifyAddress::parse("ops@example.com").unwrap());
    let changes = ProjectChanges { settings: Some(settings), ..Default::default() };
    t.use_cases.update.execute(&actor(), project.id, changes, &request()).await.unwrap();
    assert_eq!(t.use_cases.test_mail.execute(project.id).await.unwrap(), 1);
    let sent = t.mailer.0.lock().unwrap();
    assert_eq!(sent[0].to, ["ops@example.com"]);
    assert!(sent[0].subject.contains("orders"));
}
