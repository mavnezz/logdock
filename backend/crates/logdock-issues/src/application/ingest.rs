use std::sync::Arc;

use logdock_kernel::redaction::RedactionRules;
use logdock_kernel::{Scope, Timestamp, Value};

use super::alerts::{NotifyPolicy, alert_mail};
use super::{Context, IssueError, hours_ago};
use crate::domain::{ErrorEvent, IssueId, NewIssue, fingerprint};

/// What became of an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestedEvent {
    pub issue: IssueId,
    pub new: bool,
    pub regressed: bool,
    /// The event was kept already; nothing changed.
    pub duplicate: bool,
}

/// Events from the future get the time of receipt (an hour of skew is
/// allowed).
const MAX_SKEW_SECONDS: i64 = 3600;

/// Command: keeps an error event under its key's scope, groups it into its
/// issue, and tells the project's addresses about new and returning ones.
#[derive(Clone)]
pub struct IngestEvent {
    pub(crate) context: Arc<Context>,
}

impl IngestEvent {
    pub async fn execute(
        &self,
        scope: Scope,
        rules: &RedactionRules,
        policy: &NotifyPolicy,
        payload: Value,
        envelope_event_id: Option<&str>,
    ) -> Result<IngestedEvent, IssueError> {
        let c = &self.context;
        let received = c.ports.clock.now();
        let (payload, _redactions) = ErrorEvent::prepare(payload, rules);
        let event = ErrorEvent::from_payload(payload, envelope_event_id);
        if event.event_id.is_empty() {
            return Err(IssueError::NoEventId);
        }
        if let Some(issue) = c.ports.events.seen(&event.event_id, hours_ago(received, 24)).await? {
            return Ok(IngestedEvent { issue, new: false, regressed: false, duplicate: true });
        }
        let latest = Timestamp::from_unix_seconds(received.unix_seconds() + MAX_SKEW_SECONDS);
        let time = event.time.filter(|t| *t <= latest).unwrap_or(received);
        let grouping = fingerprint(&event);

        let guard = c.grouping.lock().await;
        let (issue, new, observation) = match c.ports.issues.find(scope, &grouping.hash).await? {
            Some(mut issue) => {
                let observation = issue.observe(time, event.level, event.release.as_deref(), &event.event_id);
                c.ports.issues.save(&issue, Some(&event.payload)).await?;
                (issue, false, observation)
            }
            None => {
                let new = NewIssue {
                    scope,
                    fingerprint: grouping.hash.clone(),
                    title: grouping.title.clone(),
                    culprit: grouping.culprit.clone(),
                    level: event.level,
                    seen: time,
                    release: event.release.clone(),
                    event_id: event.event_id.clone(),
                };
                match c.ports.issues.create(new, &event.payload).await? {
                    Some(issue) => (issue, true, Default::default()),
                    None => return Err(IssueError::NotFound),
                }
            }
        };
        drop(guard);
        c.ports.issues.record(issue.id, time, event.release.as_deref()).await?;
        c.ports.events.insert(issue.id, scope, &event, time, received).await?;

        let tell = (new && policy.new_issues) || (observation.regressed && policy.regressions);
        if tell && !policy.recipients.is_empty() && c.ports.mailer.configured() {
            let mail = alert_mail(policy, &issue, &event, observation.regressed, c.base_url.as_deref());
            let mailer = Arc::clone(&c.ports.mailer);
            // Ingest does not wait for the mail server.
            tokio::spawn(async move {
                if let Err(error) = mailer.send(mail).await {
                    tracing::warn!(%error, "cannot send the issue's e-mail");
                }
            });
        }
        Ok(IngestedEvent { issue: issue.id, new, regressed: observation.regressed, duplicate: false })
    }
}
