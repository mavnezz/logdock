use std::time::Duration;

use logdock_kernel::{Outcome, RequestContext, Secret};

use crate::application::commands::{
    AccountChanges, ChangePasswordInput, LoginInput, NewAccountInput, ProfileChanges, SetupInput,
};
use crate::application::{AuthContext, IdentityError};
use crate::domain::{AccountId, Locale, Role};
use crate::testing::{SETUP_CODE, TestIdentity};

const PASSWORD: &str = "correct horse battery";

fn request() -> RequestContext {
    RequestContext { client_ip: Some([192, 0, 2, 1].into()), ..Default::default() }
}

fn setup_input(code: &str) -> SetupInput {
    SetupInput {
        code: code.into(),
        email: "Ada@Example.com".into(),
        name: "Ada".into(),
        password: Secret::new(PASSWORD),
        locale: Some("de".into()),
    }
}

async fn admin(identity: &TestIdentity) -> AuthContext {
    identity.use_cases.complete_setup.execute(setup_input(SETUP_CODE), &request()).await.unwrap().session
}

async fn member(identity: &TestIdentity, auth: &AuthContext, email: &str) -> AuthContext {
    let input = NewAccountInput {
        email: email.into(),
        name: "Member".into(),
        role: Role::Member,
        password: Secret::new(PASSWORD),
        locale: None,
    };
    identity.use_cases.create_account.execute(auth, input, &request()).await.unwrap();
    login(identity, email, PASSWORD).await.unwrap()
}

async fn login(identity: &TestIdentity, email: &str, password: &str) -> Result<AuthContext, IdentityError> {
    let input = LoginInput { email: email.into(), password: Secret::new(password) };
    identity.use_cases.login.execute(input, &request()).await.map(|signed_in| signed_in.session)
}

#[tokio::test]
async fn the_first_setup_creates_an_administrator_once() {
    let identity = TestIdentity::new();
    assert!(identity.use_cases.setup_status.execute().await.unwrap());
    let wrong = identity.use_cases.complete_setup.execute(setup_input("NOPE"), &request()).await;
    assert!(matches!(wrong, Err(IdentityError::InvalidSetupCode)));

    let signed_in =
        identity.use_cases.complete_setup.execute(setup_input("test code"), &request()).await.unwrap();
    assert_eq!(signed_in.session.account.email.as_str(), "ada@example.com");
    assert_eq!(signed_in.session.account.role, Role::Admin);
    assert_eq!(signed_in.session.account.locale, Locale::De);
    assert!(!identity.use_cases.setup_status.execute().await.unwrap());

    let again = identity.use_cases.complete_setup.execute(setup_input(SETUP_CODE), &request()).await;
    assert!(matches!(again, Err(IdentityError::SetupCompleted)));
    let audit = identity.audit.0.lock().unwrap();
    assert_eq!(audit.iter().filter(|e| e.action == "setup" && e.outcome == Outcome::Success).count(), 1);
}

#[tokio::test]
async fn signs_in_and_out_and_tells_nothing_about_unknown_accounts() {
    let identity = TestIdentity::new();
    let admin = admin(&identity).await;
    identity.use_cases.logout.execute(&admin, &request()).await.unwrap();
    assert!(identity.sessions.sessions.lock().unwrap().is_empty());

    let auth = login(&identity, " ADA@example.com ", PASSWORD).await.unwrap();
    assert_eq!(auth.account.name, "Ada");
    assert!(matches!(
        login(&identity, "ada@example.com", "wrong password").await,
        Err(IdentityError::InvalidCredentials)
    ));
    assert!(matches!(
        login(&identity, "eve@example.com", PASSWORD).await,
        Err(IdentityError::InvalidCredentials)
    ));
    assert!(matches!(
        login(&identity, "not an address", PASSWORD).await,
        Err(IdentityError::InvalidCredentials)
    ));
    let reasons: Vec<_> = identity.audit.0.lock().unwrap().iter().filter_map(|e| e.reason.clone()).collect();
    assert_eq!(reasons, ["wrong_password", "unknown_account", "unknown_account"]);
}

#[tokio::test]
async fn pauses_sign_ins_after_too_many_failures() {
    let identity = TestIdentity::new();
    admin(&identity).await;
    for _ in 0..5 {
        let _ = login(&identity, "ada@example.com", "wrong password").await;
    }
    let locked = login(&identity, "ada@example.com", PASSWORD).await;
    assert!(matches!(locked, Err(IdentityError::TooManyAttempts { .. })));
    identity.clock.advance(Duration::from_secs(301));
    assert!(login(&identity, "ada@example.com", PASSWORD).await.is_ok());
}

#[tokio::test]
async fn sessions_run_out_when_idle_and_stay_alive_when_used() {
    let identity = TestIdentity::new();
    let signed_in =
        identity.use_cases.complete_setup.execute(setup_input(SETUP_CODE), &request()).await.unwrap();
    let token = signed_in.token;
    let authenticate = &identity.use_cases.authenticate;
    let auth = authenticate.execute(&token).await.unwrap();
    assert_eq!(auth.csrf_token.expose(), signed_in.session.csrf_token.expose());

    identity.clock.advance(Duration::from_secs(11 * 3600));
    authenticate.execute(&token).await.unwrap();
    identity.clock.advance(Duration::from_secs(11 * 3600));
    assert!(authenticate.execute(&token).await.is_ok(), "the use 11 hours ago kept it alive");
    identity.clock.advance(Duration::from_secs(13 * 3600));
    assert!(matches!(authenticate.execute(&token).await, Err(IdentityError::Unauthenticated)));
    assert!(identity.sessions.sessions.lock().unwrap().is_empty(), "the expired session is gone");
    assert!(matches!(
        authenticate.execute(&Secret::new("forged")).await,
        Err(IdentityError::Unauthenticated)
    ));
}

#[tokio::test]
async fn administrators_manage_accounts_members_do_not() {
    let identity = TestIdentity::new();
    let admin = admin(&identity).await;
    let member = member(&identity, &admin, "bob@example.com").await;

    let accounts = identity.use_cases.list_accounts.execute(&admin).await.unwrap();
    assert_eq!(
        accounts.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(),
        ["ada@example.com", "bob@example.com"]
    );
    assert!(matches!(identity.use_cases.list_accounts.execute(&member).await, Err(IdentityError::Forbidden)));

    let taken = NewAccountInput {
        email: "BOB@example.com".into(),
        name: "Bob".into(),
        role: Role::Member,
        password: Secret::new(PASSWORD),
        locale: None,
    };
    let result = identity.use_cases.create_account.execute(&admin, taken, &request()).await;
    assert!(matches!(result, Err(IdentityError::EmailTaken)));

    let bob = member.account.id;
    let changes =
        AccountChanges { role: Some(Role::Admin), name: Some("Robert".into()), ..Default::default() };
    let changed = identity.use_cases.update_account.execute(&admin, bob, changes, &request()).await.unwrap();
    assert_eq!((changed.role, changed.name.as_str()), (Role::Admin, "Robert"));

    let disable = AccountChanges { disabled: Some(true), ..Default::default() };
    identity.use_cases.update_account.execute(&admin, bob, disable, &request()).await.unwrap();
    assert!(
        identity.sessions.sessions.lock().unwrap().iter().all(|s| s.account_id != bob),
        "bob's sessions ended"
    );
    assert!(matches!(
        login(&identity, "bob@example.com", PASSWORD).await,
        Err(IdentityError::InvalidCredentials)
    ));

    let own = identity
        .use_cases
        .update_account
        .execute(&admin, admin.account.id, AccountChanges::default(), &request())
        .await;
    assert!(matches!(own, Err(IdentityError::OwnAccount)));
    let missing = identity.use_cases.delete_account.execute(&admin, AccountId::new(99), &request()).await;
    assert!(matches!(missing, Err(IdentityError::AccountNotFound)));
    identity.use_cases.delete_account.execute(&admin, bob, &request()).await.unwrap();
    assert_eq!(identity.use_cases.list_accounts.execute(&admin).await.unwrap().len(), 1);
}

#[tokio::test]
async fn two_administrators_cannot_demote_each_other_into_none() {
    let identity = TestIdentity::new();
    let ada = admin(&identity).await;
    let bob = member(&identity, &ada, "bob@example.com").await;
    let promote = AccountChanges { role: Some(Role::Admin), ..Default::default() };
    identity.use_cases.update_account.execute(&ada, bob.account.id, promote, &request()).await.unwrap();
    let bob = login(&identity, "bob@example.com", PASSWORD).await.unwrap();

    // Both act at once, each with the session they signed in with.
    let demote = || AccountChanges { role: Some(Role::Member), ..Default::default() };
    identity.use_cases.update_account.execute(&bob, ada.account.id, demote(), &request()).await.unwrap();
    let result = identity.use_cases.update_account.execute(&ada, bob.account.id, demote(), &request()).await;
    assert!(matches!(result, Err(IdentityError::LastAdmin)));
    let result = identity.use_cases.delete_account.execute(&ada, bob.account.id, &request()).await;
    assert!(matches!(result, Err(IdentityError::LastAdmin)));
    let disable = AccountChanges { disabled: Some(true), ..Default::default() };
    let result = identity.use_cases.update_account.execute(&ada, bob.account.id, disable, &request()).await;
    assert!(matches!(result, Err(IdentityError::LastAdmin)));

    // A disabled administrator is no administrator that remains.
    let promote = AccountChanges { role: Some(Role::Admin), ..Default::default() };
    identity.use_cases.update_account.execute(&bob, ada.account.id, promote, &request()).await.unwrap();
    let disable = AccountChanges { disabled: Some(true), ..Default::default() };
    identity.use_cases.update_account.execute(&ada, bob.account.id, disable, &request()).await.unwrap();
    identity.use_cases.delete_account.execute(&ada, bob.account.id, &request()).await.unwrap();
    assert_eq!(identity.accounts.emails(), ["ada@example.com"]);
}

#[tokio::test]
async fn people_change_their_own_password_and_profile() {
    let identity = TestIdentity::new();
    let admin = admin(&identity).await;
    let other_session = login(&identity, "ada@example.com", PASSWORD).await.unwrap();

    let wrong =
        ChangePasswordInput { current: Secret::new("not it at all"), new: Secret::new("a new password") };
    let result = identity.use_cases.change_password.execute(&admin, wrong, &request()).await;
    assert!(matches!(result, Err(IdentityError::WrongPassword)));
    let short = ChangePasswordInput { current: Secret::new(PASSWORD), new: Secret::new("short") };
    let result = identity.use_cases.change_password.execute(&admin, short, &request()).await;
    assert!(matches!(result, Err(IdentityError::PasswordRejected(_))));

    let good = ChangePasswordInput { current: Secret::new(PASSWORD), new: Secret::new("a new password") };
    identity.use_cases.change_password.execute(&admin, good, &request()).await.unwrap();
    let sessions = identity.sessions.sessions.lock().unwrap().clone();
    assert!(sessions.iter().any(|s| s.id == admin.session_id), "this session stays");
    assert!(sessions.iter().all(|s| s.id != other_session.session_id), "the other one ended");
    assert!(login(&identity, "ada@example.com", "a new password").await.is_ok());

    let changes = ProfileChanges { name: Some("Ada L.".into()), locale: Some("en".into()) };
    let profile = identity.use_cases.update_profile.execute(&admin, changes).await.unwrap();
    assert_eq!((profile.name.as_str(), profile.locale), ("Ada L.", Locale::En));
    let bad = ProfileChanges { locale: Some("fr".into()), ..Default::default() };
    assert!(identity.use_cases.update_profile.execute(&admin, bad).await.is_err());
}

#[tokio::test]
async fn administrators_reset_passwords_and_read_the_audit_log() {
    let identity = TestIdentity::new();
    let admin = admin(&identity).await;
    let member = member(&identity, &admin, "bob@example.com").await;
    identity
        .use_cases
        .reset_password
        .execute(&admin, member.account.id, Secret::new("given by admin"), &request())
        .await
        .unwrap();
    assert!(login(&identity, "bob@example.com", "given by admin").await.is_ok());
    let own = identity
        .use_cases
        .reset_password
        .execute(&admin, admin.account.id, Secret::new("x"), &request())
        .await;
    assert!(matches!(own, Err(IdentityError::OwnAccount)));

    let entries = identity.use_cases.audit_log.execute(&admin, None, 2).await.unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].1.action, "login");
    let older = identity.use_cases.audit_log.execute(&admin, Some(entries[1].0), 50).await.unwrap();
    assert!(older.iter().all(|(id, _)| *id < entries[1].0));
    let member = login(&identity, "bob@example.com", "given by admin").await.unwrap();
    assert!(matches!(
        identity.use_cases.audit_log.execute(&member, None, 10).await,
        Err(IdentityError::Forbidden)
    ));
}

#[tokio::test]
async fn purges_expired_sessions() {
    let identity = TestIdentity::new();
    admin(&identity).await;
    assert_eq!(identity.use_cases.purge_sessions.execute().await.unwrap(), 0);
    identity.clock.advance(Duration::from_secs(13 * 3600));
    assert_eq!(identity.use_cases.purge_sessions.execute().await.unwrap(), 1);
}
