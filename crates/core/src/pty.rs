use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub struct PtySession {
    child: Box<dyn Child + Send>,
    writer: Box<dyn Write + Send>,
    output: Arc<Mutex<String>>,
    read_cursor: usize,
}

impl PtySession {
    pub fn spawn_shell(cwd: &Path, env: Vec<(String, OsString)>) -> Result<Self> {
        let shell = std::env::var_os("SHELL")
            .filter(|shell| !shell.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/sh"));
        Self::spawn(shell, Vec::new(), cwd, env, 24, 80)
    }

    pub fn spawn(
        program: PathBuf,
        args: Vec<String>,
        cwd: &Path,
        env: Vec<(String, OsString)>,
        rows: u16,
        cols: u16,
    ) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("open pty")?;

        let mut command = CommandBuilder::new(&program);
        command.cwd(cwd);
        command.args(args);
        for (key, value) in env {
            command.env(key, value);
        }

        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("spawn pty command {}", program.display()))?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let writer = pair.master.take_writer().context("take pty writer")?;
        let output = Arc::new(Mutex::new(String::new()));
        let output_for_reader = Arc::clone(&output);
        thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buffer[..n]);
                        if let Ok(mut output) = output_for_reader.lock() {
                            output.push_str(&chunk);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            child,
            writer,
            output,
            read_cursor: 0,
        })
    }

    pub fn write(&mut self, input: &str) -> Result<()> {
        self.writer
            .write_all(input.as_bytes())
            .context("write to pty")?;
        self.writer.flush().context("flush pty writer")
    }

    pub fn read_available(&mut self) -> String {
        let Ok(output) = self.output.lock() else {
            return String::new();
        };
        let next = output
            .get(self.read_cursor..)
            .unwrap_or_default()
            .to_owned();
        self.read_cursor = output.len();
        next
    }

    pub fn read_until(&mut self, needle: &str, timeout: Duration) -> Result<String> {
        let deadline = Instant::now() + timeout;
        let mut collected = String::new();
        while Instant::now() < deadline {
            collected.push_str(&self.read_available());
            if collected.contains(needle) {
                return Ok(collected);
            }
            thread::sleep(Duration::from_millis(20));
        }
        anyhow::bail!("timed out waiting for PTY output containing {needle:?}: {collected:?}")
    }

    pub fn stop(&mut self) -> Result<()> {
        if self.child.try_wait().context("poll pty child")?.is_none() {
            self.child.kill().context("kill pty child")?;
        }
        let _ = self.child.wait();
        Ok(())
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
