use logdock_kernel::{Actor, EnvironmentId, ProjectId, RequestContext, Scope, Timestamp, Value};

use super::MonitorError;
use super::ports::MonitorQuery;
use crate::domain::tests_support::check_in;
use crate::domain::{CheckInStatus, Health};
use crate::testing::TestMonitors;

fn scope() -> Scope {
    Scope { project: ProjectId::new(1), environment: EnvironmentId::new(2) }
}

fn at(text: &str) -> Timestamp {
    Timestamp::parse_rfc3339(text).unwrap()
}

/// Lets spawned mail tasks run.
async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn follows_a_task_through_its_runs_and_mails_changes() {
    let t = TestMonitors::new();
    let ingest = |id: &'static str, status: &'static str| {
        let payload = check_in(id, "scheduled_nightly-report", status, "0 3 * * *");
        let use_cases = t.use_cases.clone();
        async move { use_cases.ingest.execute(scope(), &payload).await.unwrap() }
    };
    t.clock.set(at("2026-11-19T03:00:01Z"));
    let monitor = ingest("run1", "in_progress").await;
    assert_eq!(monitor.name(), "nightly-report");
    assert_eq!(monitor.health, Health::New);
    t.clock.set(at("2026-11-19T03:00:31Z"));
    let monitor = ingest("run1", "ok").await;
    assert_eq!(monitor.health, Health::Ok);
    let detail = t.use_cases.get.execute(monitor.id).await.unwrap();
    assert_eq!(detail.check_ins.len(), 1, "one run, its start and end together");
    assert_eq!(detail.check_ins[0].duration, Some(std::time::Duration::from_secs(30)));
    assert_eq!(detail.summary.next_run, Some(at("2026-11-20T03:00:00Z")));

    t.clock.set(at("2026-11-20T03:00:02Z"));
    ingest("run2", "in_progress").await;
    t.clock.set(at("2026-11-20T03:00:09Z"));
    let failed = ingest("run2", "error").await;
    assert_eq!(failed.health, Health::Failing);
    settle().await;
    {
        let mails = t.mailer.0.lock().unwrap();
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].subject, "[Logdock] nightly-report failed in orders / production");
        assert!(mails[0].text.contains("https://logs.example.com/monitors/1"));
    }

    // Nothing on 21 November: missed, after the margin.
    t.clock.set(at("2026-11-21T03:01:00Z"));
    assert_eq!(t.use_cases.sweep.execute().await.unwrap().missed, 0);
    t.clock.set(at("2026-11-21T03:05:00Z"));
    assert_eq!(t.use_cases.sweep.execute().await.unwrap().missed, 1);
    assert_eq!(t.use_cases.sweep.execute().await.unwrap().missed, 0, "counted once");
    let listed = t.use_cases.list.execute(MonitorQuery::default()).await.unwrap();
    assert_eq!(listed[0].monitor.health, Health::Missed);
    assert_eq!(listed[0].next_run, Some(at("2026-11-22T03:00:00Z")));
    settle().await;
    assert_eq!(t.mailer.0.lock().unwrap().len(), 1, "failing to missed is no news");

    t.clock.set(at("2026-11-22T03:00:01Z"));
    ingest("run3", "ok").await;
    settle().await;
    let mails = t.mailer.0.lock().unwrap();
    assert_eq!(mails.len(), 2);
    assert!(mails[1].subject.contains("runs again"), "{}", mails[1].subject);
}

#[tokio::test]
async fn a_run_that_does_not_end_times_out() {
    let t = TestMonitors::new();
    t.clock.set(at("2026-11-19T03:00:00Z"));
    let payload = check_in("long", "scheduled_import", "in_progress", "0 3 * * *");
    let monitor = t.use_cases.ingest.execute(scope(), &payload).await.unwrap();
    t.clock.set(at("2026-11-19T03:20:00Z"));
    assert_eq!(t.use_cases.sweep.execute().await.unwrap().timed_out, 0);
    t.clock.set(at("2026-11-19T03:31:00Z"));
    assert_eq!(t.use_cases.sweep.execute().await.unwrap().timed_out, 1);
    let detail = t.use_cases.get.execute(monitor.id).await.unwrap();
    assert_eq!(detail.check_ins[0].status, CheckInStatus::Timeout);
    assert_eq!(detail.summary.monitor.health, Health::Failing);
    settle().await;
    assert!(t.mailer.0.lock().unwrap()[0].subject.contains("took too long"));
}

#[tokio::test]
async fn catches_up_with_a_long_pause_over_several_sweeps() {
    let t = TestMonitors::new();
    t.clock.set(at("2026-11-19T00:00:00Z"));
    let payload = check_in("a", "scheduled_heartbeat", "ok", "* * * * *");
    t.use_cases.ingest.execute(scope(), &payload).await.unwrap();
    t.clock.set(at("2026-11-19T03:00:00Z"));
    assert_eq!(t.use_cases.sweep.execute().await.unwrap().missed, 100);
    assert_eq!(t.use_cases.sweep.execute().await.unwrap().missed, 78);
}

#[tokio::test]
async fn reads_schedules_in_their_time_zone_and_respects_the_settings() {
    let t = TestMonitors::new();
    *t.targets.0.lock().unwrap() = None;
    t.clock.set(at("2026-11-19T02:00:05Z"));
    let mut payload = check_in("a", "scheduled_backup", "error", "0 3 * * *");
    if let Value::Object(fields) = &mut payload {
        for (key, value) in fields.iter_mut() {
            if key == "monitor_config"
                && let Value::Object(config) = value
            {
                for (k, v) in config.iter_mut() {
                    if k == "timezone" {
                        *v = Value::String("Europe/Berlin".into());
                    }
                }
            }
        }
    }
    let monitor = t.use_cases.ingest.execute(scope(), &payload).await.unwrap();
    let detail = t.use_cases.get.execute(monitor.id).await.unwrap();
    assert_eq!(detail.summary.next_run, Some(at("2026-11-20T02:00:00Z")), "03:00 in Berlin");
    settle().await;
    assert!(t.mailer.0.lock().unwrap().is_empty(), "nobody to tell");
}

#[tokio::test]
async fn deletes_monitors_and_refuses_what_it_cannot_read() {
    let t = TestMonitors::new();
    let payload = check_in("a", "scheduled_x", "ok", "* * * * *");
    let monitor = t.use_cases.ingest.execute(scope(), &payload).await.unwrap();
    let actor = Actor { account_id: 1, name: "ada@example.com".into() };
    t.use_cases.delete.execute(&actor, monitor.id, &RequestContext::default()).await.unwrap();
    assert!(matches!(t.use_cases.get.execute(monitor.id).await, Err(MonitorError::NotFound)));
    assert!(matches!(
        t.use_cases.delete.execute(&actor, monitor.id, &RequestContext::default()).await,
        Err(MonitorError::NotFound)
    ));
    assert_eq!(t.audit.0.lock().unwrap().len(), 1);
    assert!(matches!(t.use_cases.ingest.execute(scope(), &Value::Null).await, Err(MonitorError::Unreadable)));
    let other = MonitorQuery { projects: vec![ProjectId::new(9)], environments: vec![] };
    assert!(t.use_cases.list.execute(other).await.unwrap().is_empty());
}
