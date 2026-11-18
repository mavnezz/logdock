use logdock_kernel::{AuditEntry, RequestContext, Secret, Timestamp};
use tokio::sync::Mutex;

use crate::application::{
    AccountSummary, AuthContext, IdentityConfig, IdentityError, IdentityPorts, SignedIn,
};
use crate::domain::{Account, LoginThrottle, NewSession, Permission, Role};

/// What the use cases share: the ports, the configuration, the sign-in
/// throttle, and the lock that serializes changes to accounts (so two
/// administrators cannot demote each other into no administrator at all).
pub(crate) struct Context {
    pub ports: IdentityPorts,
    pub config: IdentityConfig,
    pub throttle: std::sync::Mutex<LoginThrottle>,
    pub changes: Mutex<()>,
}

impl Context {
    pub fn new(ports: IdentityPorts, config: IdentityConfig) -> Self {
        let throttle = std::sync::Mutex::new(LoginThrottle::new(config.throttle));
        Self { ports, config, throttle, changes: Mutex::new(()) }
    }

    pub fn now(&self) -> Timestamp {
        self.ports.clock.now()
    }

    pub fn require(auth: &AuthContext, permission: Permission) -> Result<(), IdentityError> {
        if auth.can(permission) { Ok(()) } else { Err(IdentityError::Forbidden) }
    }

    pub async fn audit(&self, entry: AuditEntry) -> Result<(), IdentityError> {
        self.ports.audit.record(entry).await?;
        Ok(())
    }

    /// The throttle's keys of a sign-in attempt: the address it came from,
    /// and the account it was for.
    pub fn throttle_keys(request: &RequestContext, email: &str) -> [String; 2] {
        let ip = request.client_ip.map_or_else(|| "unknown".to_owned(), |ip| ip.to_string());
        [format!("ip:{ip}"), format!("account:{email}")]
    }

    pub fn check_throttle(&self, keys: &[String], now: Timestamp) -> Result<(), IdentityError> {
        let throttle = self.throttle.lock().unwrap_or_else(|e| e.into_inner());
        for key in keys {
            throttle.check(key, now).map_err(|retry_after| IdentityError::TooManyAttempts { retry_after })?;
        }
        Ok(())
    }

    pub fn record_failure(&self, keys: &[String], now: Timestamp) {
        let mut throttle = self.throttle.lock().unwrap_or_else(|e| e.into_inner());
        for key in keys {
            throttle.fail(key, now);
        }
    }

    pub fn record_success(&self, keys: &[String]) {
        let mut throttle = self.throttle.lock().unwrap_or_else(|e| e.into_inner());
        for key in keys {
            throttle.succeed(key);
        }
    }

    /// Starts a session for `account`.
    pub async fn start_session(
        &self,
        account: &Account,
        request: &RequestContext,
    ) -> Result<SignedIn, IdentityError> {
        let token = self.ports.tokens.session_token();
        let csrf_token = self.ports.tokens.csrf_token();
        let session = self
            .ports
            .sessions
            .create(NewSession {
                account_id: account.id,
                token_hash: self.ports.tokens.hash(&token),
                csrf_token: Secret::new(csrf_token.expose()),
                created_at: self.now(),
                client_ip: request.client_ip,
                user_agent: request.user_agent.clone(),
            })
            .await?;
        let expires_at = session.expires_at(&self.config.sessions);
        Ok(SignedIn {
            token,
            session: AuthContext {
                session_id: session.id,
                csrf_token,
                expires_at,
                account: AccountSummary::from(account),
            },
        })
    }

    /// Whether removing `account` from the enabled administrators would
    /// leave none.
    pub async fn is_last_admin(&self, account: &Account) -> Result<bool, IdentityError> {
        if account.role != Role::Admin || account.disabled {
            return Ok(false);
        }
        let others = self
            .ports
            .accounts
            .list()
            .await?
            .into_iter()
            .filter(|a| a.id != account.id && a.role == Role::Admin && !a.disabled)
            .count();
        Ok(others == 0)
    }
}
