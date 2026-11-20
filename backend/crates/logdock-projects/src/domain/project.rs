use std::fmt;

use logdock_kernel::performance::PerformanceRules;
use logdock_kernel::redaction::RedactionRules;
use logdock_kernel::retention::StageRetention;
use logdock_kernel::{EnvironmentId, ProjectId, Timestamp};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidProject {
    #[error("the project's name must have 1 to {max} characters")]
    Name { max: usize },
    #[error(
        "a stage's name has 1 to {max} characters: lowercase letters, digits, '.', '_' and '-', starting with a letter or digit"
    )]
    EnvironmentName { max: usize },
    #[error("{0:?} is not a valid e-mail address")]
    NotifyAddress(String),
    #[error("at most {max} addresses are notified")]
    TooManyAddresses { max: usize },
    #[error("a redaction key must have 1 to 64 characters")]
    RedactionKey,
    #[error("a stage keeps its data for 1 to 3650 days, once per stage")]
    Retention,
}

/// A project's name, such as `orders-backend`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectName(String);

impl ProjectName {
    pub const MAX_CHARS: usize = 64;

    pub fn parse(text: &str) -> Result<Self, InvalidProject> {
        let name = text.trim();
        let chars = name.chars().count();
        if chars == 0 || chars > Self::MAX_CHARS || name.chars().any(char::is_control) {
            return Err(InvalidProject::Name { max: Self::MAX_CHARS });
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A stage's name, as the SDK sends it in `environment`: `production`,
/// `staging`, `development`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnvironmentName(String);

impl EnvironmentName {
    pub const MAX_LEN: usize = 32;

    pub fn parse(text: &str) -> Result<Self, InvalidProject> {
        let name = text.trim().to_lowercase();
        let invalid = InvalidProject::EnvironmentName { max: Self::MAX_LEN };
        let first = name.chars().next().ok_or(invalid.clone())?;
        if name.len() > Self::MAX_LEN
            || !first.is_ascii_alphanumeric()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        {
            return Err(invalid);
        }
        Ok(Self(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EnvironmentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    pub id: EnvironmentId,
    pub name: EnvironmentName,
    pub created_at: Timestamp,
}

/// An address that is notified of a project's new and returning errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyAddress(String);

impl NotifyAddress {
    pub fn parse(text: &str) -> Result<Self, InvalidProject> {
        let address = text.trim().to_lowercase();
        let valid = address.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.')
        }) && address.len() <= 254
            && !address
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';'))
            && address.matches('@').count() == 1;
        if valid { Ok(Self(address)) } else { Err(InvalidProject::NotifyAddress(text.to_owned())) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a project's administrators decide about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSettings {
    /// Who gets e-mail about the project, also people without an account.
    pub notify: Vec<NotifyAddress>,
    pub notify_new_issues: bool,
    pub notify_regressions: bool,
    /// Scheduled tasks that failed or did not run (R2).
    pub notify_monitors: bool,
    pub redaction: RedactionRules,
    /// When requests, jobs and queries become performance issues.
    pub performance: PerformanceRules,
    /// Stages that keep their data for a number of days of their own.
    pub retention: Vec<(EnvironmentId, StageRetention)>,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            notify: Vec::new(),
            notify_new_issues: true,
            notify_regressions: true,
            notify_monitors: true,
            redaction: RedactionRules::default(),
            performance: PerformanceRules::default(),
            retention: Vec::new(),
        }
    }
}

impl ProjectSettings {
    pub const MAX_ADDRESSES: usize = 50;

    pub fn validate(&self) -> Result<(), InvalidProject> {
        if self.notify.len() > Self::MAX_ADDRESSES {
            return Err(InvalidProject::TooManyAddresses { max: Self::MAX_ADDRESSES });
        }
        let keys = self.redaction.extra_keys.iter().chain(self.redaction.allowed_keys.iter().flatten());
        if keys.into_iter().any(|key| key.trim().is_empty() || key.chars().count() > 64) {
            return Err(InvalidProject::RedactionKey);
        }
        let stages = &self.retention;
        let twice =
            stages.iter().enumerate().any(|(i, (id, _))| stages[..i].iter().any(|(other, _)| other == id));
        if twice || stages.iter().any(|(_, retention)| !retention.is_valid()) {
            return Err(InvalidProject::Retention);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub name: ProjectName,
    pub environments: Vec<Environment>,
    pub settings: ProjectSettings,
    pub created_at: Timestamp,
}

impl Project {
    pub fn environment(&self, id: EnvironmentId) -> Option<&Environment> {
        self.environments.iter().find(|e| e.id == id)
    }

    pub fn environment_named(&self, name: &EnvironmentName) -> Option<&Environment> {
        self.environments.iter().find(|e| &e.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_project_and_stage_names() {
        assert_eq!(ProjectName::parse(" orders-backend ").unwrap().as_str(), "orders-backend");
        assert!(ProjectName::parse("").is_err());
        assert!(ProjectName::parse(&"x".repeat(65)).is_err());
        assert_eq!(EnvironmentName::parse("Production").unwrap().as_str(), "production");
        assert_eq!(EnvironmentName::parse("eu-staging.2").unwrap().to_string(), "eu-staging.2");
        for bad in ["", "-prod", "pro duction", "prod/1", &"x".repeat(33)] {
            assert!(EnvironmentName::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn checks_settings() {
        assert_eq!(NotifyAddress::parse(" Ops@Example.com ").unwrap().as_str(), "ops@example.com");
        for bad in ["ops", "ops@example", "a@b@c.de", "x y@example.com"] {
            assert!(NotifyAddress::parse(bad).is_err(), "{bad}");
        }
        let mut settings = ProjectSettings::default();
        assert!(settings.validate().is_ok());
        settings.redaction.extra_keys = vec!["  ".into()];
        assert_eq!(settings.validate(), Err(InvalidProject::RedactionKey));
        settings.redaction.extra_keys.clear();
        let dev = (EnvironmentId::new(2), StageRetention { logs_days: Some(3), ..Default::default() });
        settings.retention = vec![dev];
        assert!(settings.validate().is_ok());
        settings.retention = vec![dev, dev];
        assert_eq!(settings.validate(), Err(InvalidProject::Retention), "once per stage");
        settings.retention = vec![(dev.0, StageRetention { traces_days: Some(0), ..Default::default() })];
        assert_eq!(settings.validate(), Err(InvalidProject::Retention));
        settings.retention.clear();
        settings.notify = vec![NotifyAddress::parse("a@b.de").unwrap(); 51];
        assert!(matches!(settings.validate(), Err(InvalidProject::TooManyAddresses { .. })));
    }

    #[test]
    fn finds_its_stages() {
        let production = Environment {
            id: EnvironmentId::new(3),
            name: EnvironmentName::parse("production").unwrap(),
            created_at: Timestamp::from_unix_seconds(0),
        };
        let project = Project {
            id: ProjectId::new(1),
            name: ProjectName::parse("orders").unwrap(),
            environments: vec![production.clone()],
            settings: ProjectSettings::default(),
            created_at: Timestamp::from_unix_seconds(0),
        };
        assert_eq!(project.environment(EnvironmentId::new(3)), Some(&production));
        assert_eq!(project.environment_named(&production.name), Some(&production));
        assert_eq!(project.environment(EnvironmentId::new(4)), None);
    }
}
