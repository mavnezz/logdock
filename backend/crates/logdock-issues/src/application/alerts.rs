//! The e-mail about new and returning issues.

use logdock_kernel::Mail;

use crate::domain::{ErrorEvent, Issue};

/// Who is told about a project's issues, and of what (its settings).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NotifyPolicy {
    pub recipients: Vec<String>,
    pub new_issues: bool,
    pub regressions: bool,
    /// For the text: `orders-backend`, `production`.
    pub project: String,
    pub environment: String,
}

/// The e-mail for an issue that is new, or came back (`regressed`).
pub fn alert_mail(
    policy: &NotifyPolicy,
    issue: &Issue,
    event: &ErrorEvent,
    regressed: bool,
    base_url: Option<&str>,
) -> Mail {
    let place = format!("{} / {}", policy.project, policy.environment);
    let headline =
        if regressed { format!("Issue came back in {place}") } else { format!("New issue in {place}") };
    let subject = format!("[Logdock] {headline}: {}", issue.title.chars().take(120).collect::<String>());
    let mut text = format!("{headline}\n\n{}\n", issue.title);
    if let Some(culprit) = &issue.culprit {
        text.push_str(&format!("{culprit}\n"));
    }
    text.push('\n');
    let mut line = |label: &str, value: Option<&str>| {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            text.push_str(&format!("{label}: {value}\n"));
        }
    };
    line("Seen", Some(&issue.last_seen.utc().to_string()));
    line("Events so far", Some(&issue.count.to_string()));
    line("Release", event.release.as_deref());
    line("Route or job", event.transaction.as_deref());
    line("Service", event.tag("service"));
    line("Host", event.tag("host"));
    line("Container", event.server_name.as_deref());
    if regressed {
        text.push_str("\nIt was resolved and has been seen again.\n");
    }
    if let Some(base) = base_url {
        text.push_str(&format!("\n{}/issues/{}\n", base.trim_end_matches('/'), issue.id.get()));
    }
    text.push_str("\n-- \nLogdock\n");
    Mail { to: policy.recipients.clone(), subject, text }
}
