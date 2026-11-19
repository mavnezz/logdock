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
use logdock_infra::issues::{SegmentEvents, SqliteIssues};
use logdock_infra::logs::{LogWriter, SegmentLogStore, SqliteLogStats, WriterConfig, WriterTask};
use logdock_infra::mail::SmtpMailer;
use logdock_infra::monitors::{BundledTimeZones, ProjectNoticeTargets, SqliteMonitors};
use logdock_infra::performance::{SegmentTransactions, SqliteRollups};
use logdock_infra::segments::{Segments, SqliteHousekeeping};
use logdock_infra::sqlite::{
    Database, RandomKeys, SqliteAccounts, SqliteAudit, SqliteIngestCounter, SqliteKeys, SqliteProjects,
    SqliteSessions,
};
use logdock_issues::application::{IssuePorts, IssueUseCases};
use logdock_kernel::{Mailer, Secret};
use logdock_logs::application::{LogPorts, LogUseCases};
use logdock_logs::domain::LogLimits;
use logdock_monitors::application::{MonitorPorts, MonitorUseCases};
use logdock_performance::application::{PerformancePorts, PerformanceUseCases};
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
    segments: Arc<Segments>,
    events: Arc<SegmentEvents>,
    transactions: Arc<SegmentTransactions>,
    rollups: Arc<SqliteRollups>,
    log_writer: WriterTask,
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

        let mailer = SmtpMailer::new(config.smtp_url.as_ref().map(Secret::expose), &config.mail_from)?;
        if !mailer.configured() {
            tracing::info!("no SMTP server configured (LOGDOCK_SMTP_URL): no e-mail is sent");
        }
        let mailer: Arc<dyn Mailer> = Arc::new(mailer);

        let counter = Arc::new(SqliteIngestCounter::new(db.clone()));
        let projects = ProjectUseCases::new(ProjectPorts {
            projects: Arc::new(SqliteProjects::new(db.clone())),
            keys: Arc::new(SqliteKeys::new(db.clone())),
            tokens: Arc::new(RandomKeys),
            counter: counter.clone(),
            audit: audit.clone(),
            mailer: mailer.clone(),
            clock: clock.clone(),
        });

        let writers = WriterHealth::default();
        let segments = Arc::new(Segments::new(config.data_dir.join("segments"), db.clone()));
        let writer_config = WriterConfig { queue: config.ingest_queue, ..WriterConfig::default() };
        let (log_writer, log_writer_task) = LogWriter::start(Arc::clone(&segments), &writers, writer_config);
        let log_writer = Arc::new(log_writer);
        let logs = LogUseCases::new(
            LogPorts {
                sink: log_writer.clone(),
                store: Arc::new(SegmentLogStore::new(Arc::clone(&segments))),
                stats: Arc::new(SqliteLogStats::new(Arc::clone(&segments))),
                feed: log_writer,
                clock: clock.clone(),
            },
            LogLimits::default(),
        );
        let events = Arc::new(SegmentEvents::new(Arc::clone(&segments)));
        let issues = IssueUseCases::new(
            IssuePorts {
                issues: Arc::new(SqliteIssues::new(db.clone())),
                events: events.clone(),
                mailer: mailer.clone(),
                audit: audit.clone(),
                clock: clock.clone(),
            },
            config.public_url.clone(),
        );
        let transactions = Arc::new(SegmentTransactions::new(Arc::clone(&segments)));
        let rollups = Arc::new(SqliteRollups::new(db.clone()));
        let performance = PerformanceUseCases::new(PerformancePorts {
            transactions: transactions.clone(),
            rollups: rollups.clone(),
            clock: clock.clone(),
        });
        let monitors = MonitorUseCases::new(
            MonitorPorts {
                monitors: Arc::new(SqliteMonitors::new(db.clone())),
                zones: Arc::new(BundledTimeZones),
                targets: Arc::new(ProjectNoticeTargets::new(db.clone())),
                mailer,
                audit: audit.clone(),
                clock: clock.clone(),
            },
            config.public_url.clone(),
        );
        let probe = DataDirProbe::new(db.clone(), config.data_dir.clone(), writers);
        let retention = RetentionUseCases::new(
            RetentionPorts {
                probe: Arc::new(probe),
                catalog: segments.clone(),
                housekeeping: Arc::new(SqliteHousekeeping::new(db.clone())),
                clock: clock.clone(),
            },
            config.retention,
        );

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
            ingest: config.ingest,
        };

        let (stop, stopped) = watch::channel(false);
        let mut tasks = Vec::new();
        let flush = counter.clone();
        tasks.push(every(Duration::from_secs(10), stopped.clone(), move || {
            let flush = flush.clone();
            async move {
                if let Err(error) = flush.flush().await {
                    tracing::warn!(error = %chain(&error), "cannot write the ingest counts");
                }
            }
        }));
        let flush = rollups.clone();
        tasks.push(every(Duration::from_secs(10), stopped.clone(), move || {
            let flush = flush.clone();
            async move {
                if let Err(error) = flush.flush().await {
                    tracing::warn!(error = %chain(&error), "cannot write the performance rollups");
                }
            }
        }));
        let sweep = monitors.sweep.clone();
        tasks.push(every(Duration::from_secs(60), stopped.clone(), move || {
            let sweep = sweep.clone();
            async move {
                match sweep.execute().await {
                    Ok(report) if report.missed > 0 || report.timed_out > 0 => tracing::info!(
                        missed = report.missed,
                        timed_out = report.timed_out,
                        "scheduled tasks did not run or took too long"
                    ),
                    Ok(_) => {}
                    Err(error) => tracing::warn!(error = %chain(&error), "cannot check the scheduled tasks"),
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
                    Err(error) => tracing::warn!(error = %chain(&error), "cannot delete expired sessions"),
                }
            }
        }));

        let apply = retention.apply.clone();
        tasks.push(every(Duration::from_secs(3600), stopped.clone(), move || {
            let apply = apply.clone();
            async move {
                match apply.execute().await {
                    Ok(report) if !report.deleted.is_empty() || report.closed > 0 => tracing::info!(
                        deleted = report.deleted.len(),
                        closed = report.closed,
                        pruned_rollups = report.pruned_rollups,
                        "retention applied"
                    ),
                    Ok(_) => {}
                    Err(error) => tracing::warn!(error = %chain(&error), "cannot apply the retention"),
                }
            }
        }));

        let use_cases = UseCases { identity, issues, logs, monitors, performance, projects, retention };
        let router = logdock_api::router(ApiState::new(use_cases, api_config));
        Ok(Self {
            router,
            db,
            counter,
            segments,
            events,
            transactions,
            rollups,
            log_writer: log_writer_task,
            stop,
            tasks,
        })
    }

    /// Finishes what is under way, within `timeout`, and closes the
    /// database.
    pub async fn shutdown(self, timeout: Duration) {
        let _ = self.stop.send(true);
        let finish = async {
            for task in self.tasks {
                let _ = task.await;
            }
            // What is queued is written before the database closes.
            self.log_writer.finish().await;
            self.events.close().await;
            self.transactions.close().await;
            if let Err(error) = self.rollups.flush().await {
                tracing::warn!(error = %chain(&error), "cannot write the performance rollups");
            }
            self.segments.close_readers().await;
            if let Err(error) = self.counter.flush().await {
                tracing::warn!(error = %chain(&error), "cannot write the ingest counts");
            }
            self.db.close().await;
        };
        if tokio::time::timeout(timeout, finish).await.is_err() {
            tracing::warn!("shutdown did not finish in time");
        }
    }
}

/// An error with its causes, as one line, for the log.
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
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
                tracing::warn!(error = %chain(&error), "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
