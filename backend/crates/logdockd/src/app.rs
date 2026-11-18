//! The composition root: plugs the adapters into the contexts' ports and
//! the use cases into the API, and runs what happens in the background.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use logdock_api::{ApiConfig, ApiState, UseCases};
use logdock_identity::application::ports::AccountRepository as _;
use logdock_identity::application::{IdentityConfig, IdentityPorts, IdentityUseCases};
use logdock_identity::domain::SetupCode;
use logdock_infra::SystemClock;
use logdock_infra::health::{DataDirProbe, WriterHealth};
use logdock_infra::identity::{Argon2Hasher, RandomTokens, new_setup_code};
use logdock_infra::sqlite::{
    Database, RandomKeys, SqliteAccounts, SqliteAudit, SqliteIngestCounter, SqliteKeys, SqliteProjects,
    SqliteSessions,
};
use logdock_kernel::Secret;
use logdock_projects::application::{ProjectPorts, ProjectUseCases};
use logdock_retention::application::{RetentionPorts, RetentionUseCases};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::config::Config;

/// Logdock, assembled and ready to serve.
pub struct App {
    pub router: axum::Router,
    db: Database,
    counter: Arc<SqliteIngestCounter>,
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl App {
    pub async fn build(config: &Config) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&config.data_dir)
            .with_context(|| format!("cannot create the data directory {}", config.data_dir.display()))?;
        let (db, report) =
            Database::open(&config.database_path()).await.context("cannot open the database")?;
        tracing::info!(
            path = %db.path().display(),
            created = report.created,
            schema_before = report.schema_before,
            schema = report.schema_after,
            "database ready"
        );
        let clock = Arc::new(SystemClock);
        let audit = Arc::new(SqliteAudit::new(db.clone()));

        // Identity, with a setup code while there is no account.
        let accounts = Arc::new(SqliteAccounts::new(db.clone()));
        let setup_code = if accounts.count().await? == 0 {
            let code = config
                .setup_code
                .clone()
                .map(|code| SetupCode::new(Secret::new(code)))
                .unwrap_or_else(new_setup_code);
            let url = config.public_url.as_deref().unwrap_or("this server");
            tracing::warn!(
                setup_code = code.expose(),
                "no account yet: open {url}/setup and create the first administrator with the setup code {}",
                code.expose()
            );
            Some(code)
        } else {
            None
        };
        let identity = IdentityUseCases::new(
            IdentityPorts {
                accounts,
                sessions: Arc::new(SqliteSessions::new(db.clone())),
                hasher: Arc::new(Argon2Hasher::new()?),
                tokens: Arc::new(RandomTokens),
                audit: audit.clone(),
                audit_reader: audit.clone(),
                clock: clock.clone(),
            },
            IdentityConfig { sessions: config.sessions, setup_code, ..IdentityConfig::default() },
        );

        let counter = Arc::new(SqliteIngestCounter::new(db.clone()));
        let projects = ProjectUseCases::new(ProjectPorts {
            projects: Arc::new(SqliteProjects::new(db.clone())),
            keys: Arc::new(SqliteKeys::new(db.clone())),
            tokens: Arc::new(RandomKeys),
            counter: counter.clone(),
            audit: audit.clone(),
            clock: clock.clone(),
        });

        let writers = WriterHealth::default();
        let probe = DataDirProbe::new(db.clone(), config.data_dir.clone(), writers);
        let retention = RetentionUseCases::new(RetentionPorts { probe: Arc::new(probe) }, config.retention);

        let web_dir = config.web_dir.clone().filter(|dir| {
            let found = dir.join("index.html").is_file();
            if !found {
                tracing::warn!(dir = %dir.display(), "no web interface found there; serving the API alone");
            }
            found
        });
        let api_config = ApiConfig {
            secure_cookies: config.secure_cookies,
            trusted_proxies: config.trusted_proxies.clone(),
            web_dir,
            public_url: config.public_url.clone(),
        };

        let (stop, stopped) = watch::channel(false);
        let mut tasks = Vec::new();
        let flush = counter.clone();
        tasks.push(every(Duration::from_secs(10), stopped.clone(), move || {
            let flush = flush.clone();
            async move {
                if let Err(error) = flush.flush().await {
                    tracing::warn!(%error, "cannot write the ingest counts");
                }
            }
        }));
        let purge = identity.purge_sessions.clone();
        tasks.push(every(Duration::from_secs(3600), stopped.clone(), move || {
            let purge = purge.clone();
            async move {
                match purge.execute().await {
                    Ok(0) => {}
                    Ok(count) => tracing::info!(count, "expired sessions deleted"),
                    Err(error) => tracing::warn!(%error, "cannot delete expired sessions"),
                }
            }
        }));

        let use_cases = UseCases { identity, projects, retention };
        let router = logdock_api::router(ApiState::new(use_cases, api_config));
        Ok(Self { router, db, counter, stop, tasks })
    }

    /// Finishes what is under way, within `timeout`, and closes the
    /// database.
    pub async fn shutdown(self, timeout: Duration) {
        let _ = self.stop.send(true);
        let finish = async {
            for task in self.tasks {
                let _ = task.await;
            }
            if let Err(error) = self.counter.flush().await {
                tracing::warn!(%error, "cannot write the ingest counts");
            }
            self.db.close().await;
        };
        if tokio::time::timeout(timeout, finish).await.is_err() {
            tracing::warn!("shutdown did not finish in time");
        }
    }
}

/// Runs `work` every `period` until told to stop; the first run is after one
/// period.
fn every<F, Fut>(period: Duration, mut stopped: watch::Receiver<bool>, work: F) -> JoinHandle<()>
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticks.tick() => work().await,
                _ = stopped.changed() => break,
            }
        }
    })
}

/// Serves until SIGTERM or Ctrl-C, then shuts down cleanly.
pub async fn serve(config: Config) -> anyhow::Result<()> {
    let app = App::build(&config).await?;
    let listener = TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.listen))?;
    tracing::info!(listen = %config.listen, version = logdock_kernel::VERSION, "Logdock is listening");
    axum::serve(listener, app.router.clone().into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("the server failed")?;
    tracing::info!("shutting down");
    app.shutdown(config.shutdown_timeout).await;
    tracing::info!("stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
