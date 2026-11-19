use std::time::Duration;

use logdock_kernel::redaction::RedactionRules;
use logdock_kernel::{Actor, EnvironmentId, ProjectId, RequestContext, Scope, Value};

use super::*;
use crate::domain::event::tests::{object, payload, s};
use crate::testing::TestIssues;

fn scope() -> Scope {
    Scope { project: ProjectId::new(1), environment: EnvironmentId::new(2) }
}

fn policy() -> NotifyPolicy {
    NotifyPolicy {
        recipients: vec!["ops@example.com".into()],
        new_issues: true,
        regressions: true,
        project: "orders".into(),
        environment: "production".into(),
    }
}

fn actor() -> Actor {
    Actor { account_id: 1, name: "ada@example.com".into() }
}

fn with_release(release: &str) -> Value {
    let Value::Object(mut fields) = payload() else { unreachable!() };
    fields.retain(|(k, _)| k != "release");
    fields.push(("release".into(), s(release)));
    Value::Object(fields)
}

async fn settle() {
    // Mail goes out in a task of its own.
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn groups_events_into_issues_and_mails_about_new_ones() {
    let t = TestIssues::new();
    let rules = RedactionRules::default();
    let first = t.use_cases.ingest.execute(scope(), &rules, &policy(), payload(), Some("e1")).await.unwrap();
    assert!(first.new && !first.regressed);
    let second =
        t.use_cases.ingest.execute(scope(), &rules, &policy(), with_release("r2"), Some("e2")).await.unwrap();
    assert_eq!(second.issue, first.issue);
    assert!(!second.new);
    settle().await;

    let sent = t.mailer.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "one mail: for the new issue only");
    assert!(sent[0].subject.starts_with("[Logdock] New issue in orders / production: QueryException"));
    assert!(sent[0].text.contains("https://logs.example.com/issues/1"));
    assert!(sent[0].text.contains("Route or job: POST /orders"));
    assert_eq!(sent[0].to, ["ops@example.com"]);

    let detail = t.use_cases.get.execute(first.issue).await.unwrap();
    assert_eq!(detail.issue.count, 2);
    assert_eq!(detail.issue.last_release.as_deref(), Some("r2"));
    assert_eq!(detail.hours.iter().map(|(_, n)| n).sum::<u64>(), 2);
    assert_eq!(detail.hours.len(), 14 * 24);
    assert_eq!(detail.releases.len(), 2);
    assert_eq!(detail.latest.as_ref().map(|e| e.release.as_deref()), Some(Some("r2")));

    let events = t.use_cases.events.execute(first.issue, None, 10).await.unwrap();
    assert_eq!(events.len(), 2);
    let event = t.use_cases.event.execute("E2").await.unwrap().unwrap();
    assert_eq!(event.issue, first.issue);
    let trace = t.use_cases.trace.execute("5b8efff798038103d269b633813fc60c").await.unwrap();
    assert_eq!(trace.len(), 2);

    // Another stage, another issue.
    let staging = Scope { environment: EnvironmentId::new(3), ..scope() };
    let other = t.use_cases.ingest.execute(staging, &rules, &policy(), payload(), Some("e3")).await.unwrap();
    assert_ne!(other.issue, first.issue);
    let listed =
        t.use_cases.list.execute(ports::IssueQuery { limit: 50, ..Default::default() }).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].last_day, 2);
}

#[tokio::test]
async fn resolved_issues_come_back_and_say_so() {
    let t = TestIssues::new();
    let rules = RedactionRules::default();
    let issue = t
        .use_cases
        .ingest
        .execute(scope(), &rules, &policy(), with_release("r1"), Some("e1"))
        .await
        .unwrap()
        .issue;
    t.use_cases
        .change
        .execute(&actor(), issue, StatusChange::ResolveInNextRelease, &RequestContext::default())
        .await
        .unwrap();
    let same =
        t.use_cases.ingest.execute(scope(), &rules, &policy(), with_release("r1"), Some("e2")).await.unwrap();
    assert!(!same.regressed, "the old release may still run");
    let back =
        t.use_cases.ingest.execute(scope(), &rules, &policy(), with_release("r2"), Some("e3")).await.unwrap();
    assert!(back.regressed);
    settle().await;
    let sent = t.mailer.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert!(sent[1].subject.contains("Issue came back"));
    assert!(sent[1].text.contains("resolved and has been seen again"));

    let muted = t
        .use_cases
        .change
        .execute(
            &actor(),
            issue,
            StatusChange::Mute { for_duration: Some(Duration::from_secs(3600)), events: None },
            &RequestContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(muted.status.name(), "muted");
    let reopened = t
        .use_cases
        .change
        .execute(&actor(), issue, StatusChange::Reopen, &RequestContext::default())
        .await
        .unwrap();
    assert_eq!(reopened.status.name(), "unresolved");
    let resolved = t
        .use_cases
        .change
        .execute(&actor(), issue, StatusChange::Resolve, &RequestContext::default())
        .await
        .unwrap();
    assert_eq!(resolved.status.name(), "resolved");
    let actions: Vec<String> = t.audit.0.lock().unwrap().iter().map(|e| e.action.clone()).collect();
    assert_eq!(actions, ["issue.resolve_next_release", "issue.mute", "issue.reopen", "issue.resolve"]);

    t.use_cases.delete.execute(&actor(), issue, &RequestContext::default()).await.unwrap();
    assert!(matches!(t.use_cases.get.execute(issue).await, Err(IssueError::NotFound)));
    let missing =
        t.use_cases.change.execute(&actor(), issue, StatusChange::Resolve, &RequestContext::default()).await;
    assert!(matches!(missing, Err(IssueError::NotFound)));
}

#[tokio::test]
async fn tells_nobody_when_the_project_says_so_and_needs_an_event_id() {
    let t = TestIssues::new();
    let rules = RedactionRules::default();
    let quiet = NotifyPolicy { new_issues: false, ..policy() };
    t.use_cases.ingest.execute(scope(), &rules, &quiet, payload(), Some("e1")).await.unwrap();
    let nobody = NotifyPolicy { recipients: Vec::new(), ..policy() };
    t.use_cases
        .ingest
        .execute(scope(), &rules, &nobody, object(vec![("message", s("other"))]), Some("e2"))
        .await
        .unwrap();
    settle().await;
    assert!(t.mailer.sent.lock().unwrap().is_empty());
    let none =
        t.use_cases.ingest.execute(scope(), &rules, &policy(), object(vec![("message", s("x"))]), None).await;
    assert!(matches!(none, Err(IssueError::NoEventId)));
    assert!(t.use_cases.event.execute("missing").await.unwrap().is_none());
}
