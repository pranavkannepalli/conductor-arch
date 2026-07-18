use std::env;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
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
        {
            self.state_dir.join("archcar.endpoint")
        }
        #[cfg(unix)]
        {
            let direct = self.state_dir.join("archcar.sock");
            if unix_socket_path_len(&direct) < UNIX_SOCKET_PATH_LIMIT {
                return direct;
            }
            archcar_short_unix_endpoint(&self.state_dir)
        }
    }

    pub fn shared_settings_path(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }
}

fn env_path(name: &str, fallback: PathBuf) -> PathBuf {
    env::var_os(name).map(PathBuf::from).unwrap_or(fallback)
}

#[cfg(unix)]
const UNIX_SOCKET_PATH_LIMIT: usize = 100;

#[cfg(unix)]
fn unix_socket_path_len(path: &std::path::Path) -> usize {
    path.as_os_str().as_bytes().len()
}

#[cfg(unix)]
fn archcar_short_unix_endpoint(state_dir: &std::path::Path) -> PathBuf {
    let name = format!("archcar-{:016x}.sock", stable_path_hash(state_dir));
    let bases = [
        env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        Some(env::temp_dir().join("archductor")),
        Some(PathBuf::from("/tmp/archductor")),
    ];
    bases
        .into_iter()
        .flatten()
        .map(|base| base.join(&name))
        .find(|path| unix_socket_path_len(path) < UNIX_SOCKET_PATH_LIMIT)
        .unwrap_or_else(|| PathBuf::from("/tmp").join(name))
}

#[cfg(unix)]
fn stable_path_hash(path: &std::path::Path) -> u64 {
    path.as_os_str()
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn archcar_endpoint_uses_state_dir_for_short_unix_paths() {
        let paths = AppPaths {
            config_dir: PathBuf::from("/tmp/archductor-config"),
            data_dir: PathBuf::from("/tmp/archductor-data"),
            state_dir: PathBuf::from("/tmp/archductor-state"),
            cache_dir: PathBuf::from("/tmp/archductor-cache"),
            database_path: PathBuf::from("/tmp/archductor-data/archductor.db"),
            logs_dir: PathBuf::from("/tmp/archductor-state/logs"),
        };

        assert_eq!(
            paths.archcar_endpoint_path(),
            PathBuf::from("/tmp/archductor-state/archcar.sock")
        );
    }

    #[cfg(unix)]
    #[test]
    fn archcar_endpoint_uses_short_stable_path_for_long_unix_paths() {
        let long_component = "very-long-path-component".repeat(8);
        let state_dir = PathBuf::from("/tmp").join(long_component).join("state");
        let paths = AppPaths {
            config_dir: state_dir.join("config"),
            data_dir: state_dir.join("data"),
            state_dir: state_dir.clone(),
            cache_dir: state_dir.join("cache"),
            database_path: state_dir.join("data/archductor.db"),
            logs_dir: state_dir.join("logs"),
        };

        let endpoint = paths.archcar_endpoint_path();

        assert!(unix_socket_path_len(&endpoint) < UNIX_SOCKET_PATH_LIMIT);
        assert!(endpoint.ends_with(format!(
            "archcar-{:016x}.sock",
            stable_path_hash(&state_dir)
        )));
        assert_eq!(endpoint, paths.archcar_endpoint_path());
    }
}
