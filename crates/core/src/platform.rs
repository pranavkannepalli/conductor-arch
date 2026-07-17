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

#[cfg(unix)]
pub fn configure_new_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
pub fn configure_new_process_group(_command: &mut Command) {}

#[cfg(unix)]
/// Sends SIGINT to the process group rooted at `pid`.
pub fn interrupt_process_group(pid: u32) -> std::io::Result<bool> {
    Command::new("kill")
        .arg("-INT")
        .arg(format!("-{pid}"))
        .status()
        .map(|status| status.success())
}

#[cfg(windows)]
/// Best-effort interruption for a Windows process tree.
///
/// Archductor does not currently attach managed providers to a Windows console
/// control group that can receive CTRL_C_EVENT, so this falls back to the same
/// non-forced tree termination used elsewhere.
pub fn interrupt_process_group(pid: u32) -> std::io::Result<bool> {
    terminate_process_tree(pid, false)
}

#[cfg(not(any(unix, windows)))]
/// Best-effort interruption for platforms without process-group support.
pub fn interrupt_process_group(_pid: u32) -> std::io::Result<bool> {
    Ok(false)
}

#[cfg(unix)]
pub fn terminate_process_group(pid: u32, force: bool) -> std::io::Result<bool> {
    let signal = if force { "-KILL" } else { "-TERM" };
    Command::new("kill")
        .arg(signal)
        .arg(format!("-{pid}"))
        .status()
        .map(|status| status.success())
}

#[cfg(windows)]
pub fn terminate_process_group(pid: u32, force: bool) -> std::io::Result<bool> {
    terminate_process_tree(pid, force)
}

#[cfg(not(any(unix, windows)))]
pub fn terminate_process_group(_pid: u32, _force: bool) -> std::io::Result<bool> {
    Ok(false)
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

    #[test]
    fn provider_process_group_interrupt_reaches_child_process() {
        #[cfg(unix)]
        {
            let temp = tempfile::tempdir().unwrap();
            let marker = temp.path().join("interrupted");
            let script = format!(
                "trap '' INT; (trap 'echo child > {} ; exit 0' INT; while true; do sleep 1; done) & wait",
                marker.display()
            );
            let mut command = shell_command(&script);
            configure_new_process_group(&mut command);
            let mut child = command.spawn().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(100));

            assert!(interrupt_process_group(child.id()).unwrap());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::time::Instant::now() < deadline && !marker.exists() {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let terminated = terminate_process_group(child.id(), true).unwrap_or(false);
            let exited = wait_for_child_exit(&mut child, std::time::Duration::from_secs(2));

            assert!(
                exited,
                "test process did not exit after force termination; terminate_process_group={terminated}"
            );
            assert!(marker.exists());
        }
    }

    #[cfg(unix)]
    fn wait_for_child_exit(child: &mut std::process::Child, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        false
    }
}
