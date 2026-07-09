use anyhow::{Context, Result};
use linux_archductor_core::paths::AppPaths;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;

static LOGGER_GUARDS: OnceLock<LoggerGuards> = OnceLock::new();
static PTY_SCREEN_LOG: OnceLock<PathBuf> = OnceLock::new();

struct LoggerGuards {
    _json: tracing_appender::non_blocking::WorkerGuard,
    _text: tracing_appender::non_blocking::WorkerGuard,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LoggerConfig {
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default = "default_level")]
    level: String,
    #[serde(default = "default_targets")]
    targets: Vec<String>,
    #[serde(default = "default_json")]
    json: bool,
    #[serde(default = "default_text")]
    text: bool,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            level: default_level(),
            targets: default_targets(),
            json: default_json(),
            text: default_text(),
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_level() -> String {
    "info".to_owned()
}

fn default_targets() -> Vec<String> {
    vec![
        "linux_archductor_gtk::session_surface=debug".to_owned(),
        "linux_archductor_gtk::terminal=debug".to_owned(),
        "linux_archductor_core::pty=debug".to_owned(),
    ]
}

fn default_json() -> bool {
    true
}

fn default_text() -> bool {
    true
}

pub(crate) fn init_dev_logger(paths: &AppPaths) -> Result<()> {
    if LOGGER_GUARDS.get().is_some() {
        return Ok(());
    }

    let config_path = paths.config_dir.join("logger.toml");
    ensure_default_config(&config_path)?;
    let config = load_config(&config_path)?;

    if !config.enabled {
        return Ok(());
    }

    fs::create_dir_all(&paths.logs_dir)
        .with_context(|| format!("create logs dir {}", paths.logs_dir.display()))?;

    let json_file = open_log_file(paths.logs_dir.join("gtk-dev.jsonl"))?;
    let text_file = open_log_file(paths.logs_dir.join("gtk-dev.log"))?;
    let _ = PTY_SCREEN_LOG.set(paths.logs_dir.join("pty-screens.log"));
    let (json_writer, json_guard) = tracing_appender::non_blocking(json_file);
    let (text_writer, text_guard) = tracing_appender::non_blocking(text_file);
    let filter = build_filter(&config);

    let json_layer = fmt::layer()
        .with_writer(json_writer)
        .with_ansi(false)
        .json()
        .with_current_span(true)
        .with_span_list(true)
        .with_filter(filter.clone());
    let text_layer = fmt::layer()
        .with_writer(text_writer)
        .with_ansi(false)
        .with_filter(filter);

    tracing_subscriber::registry()
        .with(json_layer)
        .with(text_layer)
        .try_init()
        .context("initialize tracing subscriber")?;

    tracing::info!(
        config_path = %config_path.display(),
        logs_dir = %paths.logs_dir.display(),
        level = %config.level,
        targets = ?config.targets,
        "dev logger initialized"
    );

    let _ = LOGGER_GUARDS.set(LoggerGuards {
        _json: json_guard,
        _text: text_guard,
    });

    Ok(())
}

fn build_filter(config: &LoggerConfig) -> EnvFilter {
    let mut directives = vec![config.level.clone()];
    directives.extend(config.targets.iter().cloned());
    EnvFilter::builder().parse_lossy(directives.join(","))
}

fn ensure_default_config(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create logger config dir {}", parent.display()))?;
    }
    let body = toml::to_string_pretty(&LoggerConfig::default())
        .context("serialize default logger config")?;
    fs::write(path, body).with_context(|| format!("write {}", path.display()))
}

fn load_config(path: &Path) -> Result<LoggerConfig> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

fn open_log_file(path: PathBuf) -> Result<std::fs::File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))
}

pub(crate) fn write_pty_screen_snapshot(source: &str, process_id: i64, screen: &str) {
    if !pty_screen_snapshot_logging_enabled() {
        return;
    }
    let Some(path) = PTY_SCREEN_LOG.get() else {
        return;
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = writeln!(
        file,
        "=== [unix_ms={ts}] source={source} process_id={process_id} ===\n{}\n===",
        screen.trim_end_matches('\n')
    );
}

fn pty_screen_snapshot_logging_enabled() -> bool {
    linux_archductor_core::env_flags::enabled("ARCHDUCTOR_LOG_PTY_SCREENS")
}
