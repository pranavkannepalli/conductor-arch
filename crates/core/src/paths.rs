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
        let home = crate::platform::home_dir().unwrap_or_else(|| PathBuf::from("."));

        #[cfg(windows)]
        let (config_dir, data_dir, state_dir, cache_dir) = {
            let roaming = env_path("APPDATA", home.join("AppData/Roaming"));
            let local = env_path("LOCALAPPDATA", home.join("AppData/Local"));
            let data = local.join("Archductor");
            (
                roaming.join("Archductor"),
                data.clone(),
                data.join("state"),
                data.join("cache"),
            )
        };

        #[cfg(not(windows))]
        let config_dir = env_path("XDG_CONFIG_HOME", home.join(".config")).join("archductor");
        #[cfg(not(windows))]
        let data_dir = env_path("XDG_DATA_HOME", home.join(".local/share")).join("archductor");
        #[cfg(not(windows))]
        let state_dir = env_path("XDG_STATE_HOME", home.join(".local/state")).join("archductor");
        #[cfg(not(windows))]
        let cache_dir = env_path("XDG_CACHE_HOME", home.join(".cache")).join("archductor");

        Self {
            database_path: data_dir.join("archductor.db"),
            logs_dir: state_dir.join("logs"),
            config_dir,
            data_dir,
            state_dir,
            cache_dir,
        }
    }

    pub fn archcar_endpoint_path(&self) -> PathBuf {
        #[cfg(windows)]
        let name = "archcar.endpoint";
        #[cfg(not(windows))]
        let name = "archcar.sock";
        self.state_dir.join(name)
    }
}

fn env_path(name: &str, fallback: PathBuf) -> PathBuf {
    env::var_os(name).map(PathBuf::from).unwrap_or(fallback)
}
