use std::path::PathBuf;
use std::process::Command;

pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

pub fn shell_program() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::var_os("COMSPEC")
            .filter(|shell| !shell.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cmd.exe"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("SHELL")
            .filter(|shell| !shell.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/sh"))
    }
}

pub fn shell_command(script: &str) -> Command {
    let mut command = Command::new(shell_program());
    #[cfg(windows)]
    command.args(["/D", "/S", "/C", script]);
    #[cfg(not(windows))]
    command.args(["-c", script]);
    command
}

pub fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        let filter = format!("PID eq {pid}");
        Command::new("tasklist.exe")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| {
                let stdout = String::from_utf8_lossy(&output.stdout);
                !stdout.contains("No tasks are running")
                    && stdout
                        .lines()
                        .any(|line| line.contains(&format!("\"{pid}\"")))
            })
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

#[cfg(windows)]
pub fn terminate_process_tree(pid: u32, force: bool) -> std::io::Result<bool> {
    let mut command = Command::new("taskkill.exe");
    command.args(["/PID", &pid.to_string(), "/T"]);
    if force {
        command.arg("/F");
    }
    command.status().map(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_program_has_a_platform_default() {
        assert!(!shell_program().as_os_str().is_empty());
    }

    #[test]
    fn shell_command_accepts_a_script() {
        let command = shell_command("echo archductor");
        assert!(!command.get_program().is_empty());
        assert!(!command.get_args().collect::<Vec<_>>().is_empty());
        #[cfg(windows)]
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["/D", "/S", "/C", "echo archductor"]
        );
    }
}
