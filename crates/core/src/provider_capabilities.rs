use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const CAPABILITY_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub fn help_advertises_bare(help: &str) -> bool {
    help.split_whitespace().any(|token| {
        token.trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '-')
            == "--bare"
    })
}

pub fn executable_supports_bare(program: &str) -> bool {
    let mut command = Command::new(program);
    command.arg("--help");
    command_supports_bare(&mut command, CAPABILITY_PROBE_TIMEOUT)
}

fn command_supports_bare(command: &mut Command, timeout: Duration) -> bool {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let Ok(output) = child.wait_with_output() else {
                    return false;
                };
                let mut help = String::from_utf8_lossy(&output.stdout).into_owned();
                help.push_str(&String::from_utf8_lossy(&output.stderr));
                return help_advertises_bare(&help);
            }
            Ok(Some(_)) | Err(_) => return false,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

pub fn add_bare_when_supported(args: &mut Vec<String>, program: &str) {
    if executable_supports_bare(program) {
        args.push("--bare".to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn bare_capability_requires_an_explicit_long_option() {
        assert!(help_advertises_bare(
            "Usage: agent [OPTIONS]\n  --bare  Start without extras"
        ));
        assert!(!help_advertises_bare("bare startup mode"));
        assert!(!help_advertises_bare("--barely-there"));
    }

    #[test]
    fn capability_probe_times_out_and_terminates_a_hung_process() {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 30"]);
            command
        };
        let started = Instant::now();

        assert!(!command_supports_bare(
            &mut command,
            Duration::from_millis(50)
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
