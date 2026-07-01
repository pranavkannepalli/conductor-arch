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

        let config_dir = env_path("XDG_CONFIG_HOME", home.join(".config")).join("linux-archductor");
        let data_dir =
            env_path("XDG_DATA_HOME", home.join(".local/share")).join("linux-archductor");
        let state_dir =
            env_path("XDG_STATE_HOME", home.join(".local/state")).join("linux-archductor");
        let cache_dir = env_path("XDG_CACHE_HOME", home.join(".cache")).join("linux-archductor");

        Self {
            database_path: data_dir.join("linux-archductor.db"),
            logs_dir: state_dir.join("logs"),
            config_dir,
            data_dir,
            state_dir,
            cache_dir,
        }
    }

    pub fn archcar_socket_path(&self) -> PathBuf {
        self.state_dir.join("archcar.sock")
    }
}

fn env_path(name: &str, fallback: PathBuf) -> PathBuf {
    env::var_os(name).map(PathBuf::from).unwrap_or(fallback)
}
