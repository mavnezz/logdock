use logdock_kernel::{EnvironmentId, ProjectId, Scope, Timestamp};

use super::ports::{RollupQuery, SampleQuery};
use super::{Order, PerformanceError};
use crate::domain::Kind;
use crate::domain::tests_support::request;
use crate::testing::TestPerformance;

fn scope() -> Scope {
    Scope { project: ProjectId::new(1), environment: EnvironmentId::new(2) }
}

fn rules() -> logdock_kernel::redaction::RedactionRules {
    logdock_kernel::redaction::RedactionRules::default()
}

fn range(kind: Kind) -> RollupQuery {
    RollupQuery {
        projects: vec![],
        environments: vec![],
        kind,
        from: Timestamp::from_unix_seconds(1_795_050_000),
        to: Timestamp::from_unix_seconds(1_795_051_000),
    }
}

#[tokio::test]
async fn counts_requests_and_what_they_spent_their_time_on() {
    let t = TestPerformance::new();
    for (status, duration) in [(200, 0.1), (200, 0.3), (500, 2.0)] {
        t.use_cases
            .ingest
            .execute(scope(), &rules(), request("/orders/{order}", status, duration), None)
            .await
            .unwrap();
    }
    t.use_cases.ingest.execute(scope(), &rules(), request("/", 200, 0.01), Some(0.1)).await.unwrap();

    let requests = t.use_cases.overview.execute(range(Kind::Request), Order::Count, 50).await.unwrap();
    assert_eq!(requests[0].name, "GET /", "sampled at 10 %: it stands for ten");
    assert_eq!(requests[0].rollup.count, 10);
    assert_eq!(requests[1].name, "GET /orders/{order}");
    assert_eq!(requests[1].rollup.count, 3);
    assert_eq!(requests[1].rollup.flagged, 1);
    let by_time = t.use_cases.overview.execute(range(Kind::Request), Order::TotalTime, 50).await.unwrap();
    assert_eq!(by_time[0].name, "GET /orders/{order}");
    for order in [Order::Slowest, Order::Failures] {
        let rows = t.use_cases.overview.execute(range(Kind::Request), order, 1).await.unwrap();
        assert_eq!(rows.len(), 1);
    }

    let queries = t.use_cases.overview.execute(range(Kind::Query), Order::TotalTime, 50).await.unwrap();
    assert_eq!(queries[0].name, "select * from orders where id = ? and status in (...)");
    assert_eq!(queries[0].rollup.count, 13);
    let outgoing = t.use_cases.overview.execute(range(Kind::Outgoing), Order::TotalTime, 50).await.unwrap();
    assert_eq!(outgoing[0].rollup.flagged, 13, "502 from the provider");

    let series = t.use_cases.series.execute(range(Kind::Request), Some("GET /orders/{order}")).await.unwrap();
    assert_eq!(series.bucket_seconds, 60);
    assert_eq!(series.total.count, 3);
    assert!(series.buckets.len() >= 16);
    let all = t.use_cases.series.execute(range(Kind::Request), None).await.unwrap();
    assert!(all.total.count > series.total.count, "every request: {}", all.total.count);

    let slowest = t
        .use_cases
        .samples
        .execute(SampleQuery {
            projects: vec![],
            environments: vec![],
            kind: Kind::Request,
            name: "GET /orders/{order}".into(),
            from: Timestamp::from_unix_seconds(0),
            to: Timestamp::from_unix_seconds(2_000_000_000),
            slowest: true,
            limit: 2,
        })
        .await
        .unwrap();
    assert_eq!(slowest.len(), 2);
    assert_eq!(slowest[0].duration_us, 2_000_000);
    assert!(slowest[0].failed);

    let trace = t.use_cases.trace.execute("414460eb7b5c4bc1913034f6998e4ce8").await.unwrap();
    assert_eq!(trace.len(), 4);
    assert_eq!(trace[0].transaction.spans.len(), 4);
}

#[tokio::test]
async fn refuses_what_it_cannot_read_or_answer() {
    let t = TestPerformance::new();
    let unreadable = logdock_kernel::Value::Object(vec![]);
    assert!(matches!(
        t.use_cases.ingest.execute(scope(), &rules(), unreadable, None).await,
        Err(PerformanceError::Unreadable)
    ));
    let mut backwards = range(Kind::Request);
    std::mem::swap(&mut backwards.from, &mut backwards.to);
    assert!(matches!(
        t.use_cases.overview.execute(backwards, Order::Count, 10).await,
        Err(PerformanceError::InvalidQuery(_))
    ));
    let mut long = range(Kind::Request);
    long.from = Timestamp::from_unix_seconds(0);
    assert!(t.use_cases.series.execute(long, Some("x")).await.is_err());
    let query = SampleQuery {
        projects: vec![],
        environments: vec![],
        kind: Kind::Query,
        name: "x".into(),
        from: Timestamp::from_unix_seconds(0),
        to: Timestamp::from_unix_seconds(1),
        slowest: false,
        limit: 10,
    };
    assert!(t.use_cases.samples.execute(query).await.is_err());
}

#[tokio::test]
async fn moves_transactions_from_the_future_to_now() {
    let t = TestPerformance::new();
    let mut payload = request("/later", 200, 0.1);
    if let logdock_kernel::Value::Object(fields) = &mut payload {
        for (key, value) in fields.iter_mut() {
            if key == "start_timestamp" {
                *value = logdock_kernel::Value::Float(1_900_000_000.0);
            }
            if key == "timestamp" {
                *value = logdock_kernel::Value::Float(1_900_000_000.1);
            }
        }
    }
    let kept = t.use_cases.ingest.execute(scope(), &rules(), payload, None).await.unwrap();
    assert_eq!(kept.start, Timestamp::from_unix_seconds(1_795_050_600));
}

#[tokio::test]
async fn redacts_what_spans_carry() {
    let t = TestPerformance::new();
    let mut payload = request("/pay", 200, 0.1);
    if let logdock_kernel::Value::Object(fields) = &mut payload {
        fields.retain(|(k, _)| k != "spans");
        fields.push((
            "spans".into(),
            logdock_kernel::Value::Array(vec![logdock_kernel::Value::Object(vec![
                ("op".into(), logdock_kernel::Value::String("http.client".into())),
                (
                    "description".into(),
                    logdock_kernel::Value::String("GET https://api.example.com/v1?token=s3cr3tvalue".into()),
                ),
                ("start_timestamp".into(), logdock_kernel::Value::Float(1_795_050_499.0)),
                ("timestamp".into(), logdock_kernel::Value::Float(1_795_050_499.1)),
                (
                    "data".into(),
                    logdock_kernel::Value::Object(vec![(
                        "authorization".into(),
                        logdock_kernel::Value::String("Bearer abc.def.ghi".into()),
                    )]),
                ),
            ])]),
        ));
    }
    let kept = t.use_cases.ingest.execute(scope(), &rules(), payload, None).await.unwrap();
    let span = kept.spans.iter().find(|s| s.op == "http.client").unwrap();
    assert!(!span.description.as_deref().unwrap().contains("s3cr3tvalue"), "{:?}", span.description);
    assert_eq!(span.data.get("authorization").and_then(logdock_kernel::Value::as_str), Some("[redacted]"));
}
