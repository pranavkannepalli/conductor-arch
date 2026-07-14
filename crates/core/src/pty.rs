use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, trace, warn};
use vt100::Parser;

pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send>,
    writer: Box<dyn Write + Send>,
    output: Arc<Mutex<Vec<u8>>>,
    screen: Arc<Mutex<Parser>>,
    read_cursor: usize,
}

impl PtySession {
    pub fn spawn_shell(cwd: &Path, env: Vec<(String, OsString)>) -> Result<Self> {
        let shell = crate::platform::shell_program();
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
        debug!(
            program = %program.display(),
            cwd = %cwd.display(),
            args = ?args,
            env_count = env.len(),
            rows,
            cols,
            "spawning pty session"
        );
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
        let output = Arc::new(Mutex::new(Vec::new()));
        let screen = Arc::new(Mutex::new(Parser::new(rows, cols, 0)));
        let output_for_reader = Arc::clone(&output);
        let screen_for_reader = Arc::clone(&screen);
        thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        trace!(bytes = n, "pty read chunk");
                        if let Ok(mut output) = output_for_reader.lock() {
                            output.extend_from_slice(&buffer[..n]);
                        }
                        if let Ok(mut screen) = screen_for_reader.lock() {
                            screen.process(&buffer[..n]);
                        }
                    }
                    Err(err) => {
                        warn!(error = %err, "pty reader stopped");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            master: pair.master,
            child,
            writer,
            output,
            screen,
            read_cursor: 0,
        })
    }

    pub fn write(&mut self, input: &str) -> Result<()> {
        self.write_bytes(input.as_bytes())
    }

    pub fn write_bytes(&mut self, input: &[u8]) -> Result<()> {
        trace!(bytes = input.len(), "writing to pty");
        self.writer.write_all(input).context("write to pty")?;
        self.writer.flush().context("flush pty writer")
    }

    pub fn send_line(&mut self, input: &str) -> Result<()> {
        let bytes = crate::codex_tui::encode_send_line(input);
        let (line, enter) = bytes.split_at(bytes.len().saturating_sub(1));
        self.write_bytes(line)?;
        thread::sleep(Duration::from_millis(20));
        self.write_bytes(enter)
    }

    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        trace!(rows, cols, "resizing pty");
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("resize pty")?;
        if let Ok(mut screen) = self.screen.lock() {
            screen.screen_mut().set_size(rows, cols);
        }
        Ok(())
    }

    pub fn has_exited(&mut self) -> Result<bool> {
        Ok(self.child.try_wait().context("poll pty child")?.is_some())
    }

    pub fn read_available(&mut self) -> String {
        let Ok(output) = self.output.lock() else {
            return String::new();
        };
        let next = output.get(self.read_cursor..).unwrap_or_default().to_vec();
        self.read_cursor = output.len();
        if !next.is_empty() {
            trace!(bytes = next.len(), "drained pty output");
        }
        String::from_utf8_lossy(&next).into_owned()
    }

    pub fn visible_screen_text(&self) -> String {
        let Ok(screen) = self.screen.lock() else {
            return String::new();
        };
        screen.screen().contents()
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
        debug!(pid = self.process_id(), "stopping pty session");
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
