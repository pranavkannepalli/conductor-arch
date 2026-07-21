use anyhow::{Context, Result};
use std::ffi::OsString;
use std::process::{Child, Command};
#[cfg(not(windows))]
use std::thread;
#[cfg(not(windows))]
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct CommandSpec {
    program: OsString,
    args: Vec<OsString>,
}

impl CommandSpec {
    pub fn new<P, I, A>(program: P, args: I) -> Self
    where
        P: Into<OsString>,
        I: IntoIterator<Item = A>,
        A: Into<OsString>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

pub struct DevCommands {
    pub archcar: CommandSpec,
    pub gtk: CommandSpec,
}

struct OwnedChild {
    child: Child,
    stopped: bool,
}

impl OwnedChild {
    fn spawn(spec: &CommandSpec) -> Result<Self> {
        let mut command = Command::new(&spec.program);
        command.args(&spec.args);
        archductor_core::platform::configure_new_process_group(&mut command);
        let child = command
            .spawn()
            .with_context(|| format!("failed to start {}", spec.program.to_string_lossy()))?;
        Ok(Self {
            child,
            stopped: false,
        })
    }

    fn id(&self) -> u32 {
        self.child.id()
    }

    fn stop(&mut self) -> Result<()> {
        if self.stopped {
            return Ok(());
        }
        let pid = self.id();
        #[cfg(windows)]
        {
            let _ = archductor_core::platform::terminate_process_group(pid, true);
            let _ = self.child.wait();
            self.stopped = true;
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = archductor_core::platform::terminate_process_group(pid, false);
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if self.child.try_wait()?.is_some() {
                    self.stopped = true;
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(25));
            }
            let _ = archductor_core::platform::terminate_process_group(pid, true);
            let _ = self.child.wait();
            self.stopped = true;
            Ok(())
        }
    }
}

pub struct DevSession {
    commands: DevCommands,
    archcar: OwnedChild,
    gtk: OwnedChild,
}

impl DevSession {
    pub fn start(commands: DevCommands) -> Result<Self> {
        let archcar = OwnedChild::spawn(&commands.archcar)?;
        match OwnedChild::spawn(&commands.gtk) {
            Ok(gtk) => Ok(Self {
                commands,
                archcar,
                gtk,
            }),
            Err(error) => {
                let mut archcar = archcar;
                let _ = archcar.stop();
                Err(error)
            }
        }
    }

    pub fn archcar_pid(&self) -> u32 {
        self.archcar.id()
    }

    pub fn gtk_pid(&self) -> u32 {
        self.gtk.id()
    }

    pub fn reload_gtk(&mut self) -> Result<()> {
        self.gtk.stop()?;
        self.gtk = OwnedChild::spawn(&self.commands.gtk)?;
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<()> {
        let gtk_result = self.gtk.stop();
        let archcar_result = self.archcar.stop();
        gtk_result.and(archcar_result)
    }
}

impl Drop for DevSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandSpec, DevCommands, DevSession};

    fn long_command() -> CommandSpec {
        #[cfg(windows)]
        return CommandSpec::new("cmd.exe", ["/D", "/C", "ping -n 30 127.0.0.1 >NUL"]);
        #[cfg(not(windows))]
        return CommandSpec::new("sh", ["-c", "sleep 30"]);
    }

    #[test]
    fn reload_replaces_only_gtk_and_shutdown_stops_both() {
        let commands = DevCommands {
            archcar: long_command(),
            gtk: long_command(),
        };
        let mut session = DevSession::start(commands).unwrap();
        let archcar_pid = session.archcar_pid();
        let gtk_pid = session.gtk_pid();

        session.reload_gtk().unwrap();

        assert_eq!(session.archcar_pid(), archcar_pid);
        assert_ne!(session.gtk_pid(), gtk_pid);
        session.shutdown().unwrap();
        assert!(!archductor_core::platform::process_alive(archcar_pid));
        assert!(!archductor_core::platform::process_alive(session.gtk_pid()));
    }
}
