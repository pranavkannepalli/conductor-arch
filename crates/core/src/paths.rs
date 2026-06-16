use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub database_path: PathBuf,
    pub logs_dir: PathBuf,
}

impl AppPaths {
    pub fn from_env() -> Self {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));

        let config_dir = env_path("XDG_CONFIG_HOME", home.join(".config")).join("linux-conductor");
        let data_dir = env_path("XDG_DATA_HOME", home.join(".local/share")).join("linux-conductor");
        let state_dir =
            env_path("XDG_STATE_HOME", home.join(".local/state")).join("linux-conductor");
        let cache_dir = env_path("XDG_CACHE_HOME", home.join(".cache")).join("linux-conductor");

        Self {
            database_path: data_dir.join("linux-conductor.db"),
            logs_dir: state_dir.join("logs"),
            config_dir,
            data_dir,
            state_dir,
            cache_dir,
        }
    }
}

fn env_path(name: &str, fallback: PathBuf) -> PathBuf {
    env::var_os(name).map(PathBuf::from).unwrap_or(fallback)
}
