use std::fs::{self, OpenOptions};

use anyhow::{Context, Result};
use linux_archductor_core::archcar::server::{
    reconcile_managed_sessions_on_startup, ArchcarServer,
};
use linux_archductor_core::paths::AppPaths;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;

fn main() -> Result<()> {
    let paths = AppPaths::from_env();
    let _log_guard = init_logger(&paths)?;
    reconcile_managed_sessions_on_startup(&paths)?;
    let server = ArchcarServer::bind(paths)?;
    server.serve()
}

fn init_logger(paths: &AppPaths) -> Result<WorkerGuard> {
    fs::create_dir_all(&paths.logs_dir)
        .with_context(|| format!("create archcar logs dir {}", paths.logs_dir.display()))?;
    let log_path = paths.logs_dir.join("archcar.log");
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open archcar log {}", log_path.display()))?;
    let (writer, guard) = tracing_appender::non_blocking(log_file);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::builder()
            .with_default_directive(LevelFilter::INFO.into())
            .from_env_lossy()
    });
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_filter(filter);
    tracing_subscriber::registry().with(layer).try_init()?;
    tracing::info!(log_path = %log_path.display(), "archcar logger initialized");
    Ok(guard)
}
