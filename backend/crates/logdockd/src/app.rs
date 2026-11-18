//! The composition root: plugs the adapters into the contexts' ports and
//! the use cases into the API.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use logdock_api::{ApiConfig, ApiState, UseCases};
use logdock_infra::health::{DataDirProbe, WriterHealth};
use logdock_infra::sqlite::Database;
use logdock_retention::application::{RetentionPorts, RetentionUseCases};
use tokio::net::TcpListener;

use crate::config::Config;

/// Logdock, assembled and ready to serve.
pub struct App {
    pub router: axum::Router,
    db: Database,
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
        let router = logdock_api::router(ApiState::new(UseCases { retention }, api_config));
        Ok(Self { router, db })
    }

    /// Finishes what is under way, within `timeout`, and closes the
    /// database.
    pub async fn shutdown(self, timeout: Duration) {
        if tokio::time::timeout(timeout, self.db.close()).await.is_err() {
            tracing::warn!("the database did not close in time");
        }
    }
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
    #[cfg(unix)]
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
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
