/// What a signed-in account may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Read logs, issues, monitoring and projects (all projects and stages,
    /// decided for R1; docs/architecture.md, 7).
    ReadData,
    /// Resolve, mute and reopen issues.
    TriageIssues,
    /// Create and change projects, stages, keys and their settings.
    ManageProjects,
    /// Create, change and delete accounts.
    ManageAccounts,
    /// Read the audit log and the system's settings.
    Administer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    Member,
}

impl Role {
    pub fn grants(self, permission: Permission) -> bool {
        match self {
            Self::Admin => true,
            Self::Member => matches!(permission, Permission::ReadData | Permission::TriageIssues),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Member => "member",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_read_and_triage_admins_do_everything() {
        let all = [
            Permission::ReadData,
            Permission::TriageIssues,
            Permission::ManageProjects,
            Permission::ManageAccounts,
            Permission::Administer,
        ];
        assert!(all.iter().all(|p| Role::Admin.grants(*p)));
        let member: Vec<_> = all.iter().filter(|p| Role::Member.grants(**p)).collect();
        assert_eq!(member, [&Permission::ReadData, &Permission::TriageIssues]);
        for role in [Role::Admin, Role::Member] {
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("root"), None);
    }
}
