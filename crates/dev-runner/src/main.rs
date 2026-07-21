use anyhow::Result;
use archductor_dev_runner::process::{CommandSpec, DevCommands, DevSession};
use archductor_dev_runner::{action_for_key, DevAction, DevKey};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{self, BufReader, IsTerminal, Read};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

struct RawModeGuard(bool);

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.0 {
            let _ = disable_raw_mode();
        }
    }
}

fn main() -> Result<()> {
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal_flag = interrupted.clone();
    ctrlc::set_handler(move || signal_flag.store(true, Ordering::SeqCst))?;

    let commands = DevCommands {
        archcar: CommandSpec::new(required_env("ARCHDUCTOR_ARCHCAR_BIN")?, [] as [&str; 0]),
        gtk: CommandSpec::new(required_env("ARCHDUCTOR_GTK_BIN")?, [] as [&str; 0]),
    };
    let mut session = DevSession::start(commands)?;
    println!(
        "Archcar PID {} | GTK PID {}",
        session.archcar_pid(),
        session.gtk_pid()
    );
    let use_raw_mode = io::stdin().is_terminal();
    if use_raw_mode {
        enable_raw_mode()?;
    }
    let _raw_mode = RawModeGuard(use_raw_mode);

    println!("r Reload GTK | q Quit");
    if !use_raw_mode {
        let stdin = io::stdin();
        let reader = BufReader::new(stdin.lock());
        for byte in reader.bytes() {
            if run_action(&mut session, byte? as char)? {
                return Ok(());
            }
        }
        session.shutdown()?;
        return Ok(());
    }

    loop {
        if interrupted.load(Ordering::SeqCst) {
            session.shutdown()?;
            anyhow::bail!("interrupted");
        }
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let KeyCode::Char(character) = key.code else {
            continue;
        };
        if character == 'c' && key.modifiers.contains(KeyModifiers::CONTROL) {
            session.shutdown()?;
            anyhow::bail!("interrupted");
        }
        if run_action(&mut session, character)? {
            return Ok(());
        }
    }
}

fn run_action(session: &mut DevSession, character: char) -> Result<bool> {
    match action_for_key(DevKey::Character(character)) {
        DevAction::ReloadGtk => {
            println!("Reloading GTK...");
            let archcar_pid = session.archcar_pid();
            let previous_gtk_pid = session.gtk_pid();
            let status = Command::new("cargo")
                .args(["build", "--bin", "archductor-gtk"])
                .status()?;
            if !status.success() {
                anyhow::bail!("GTK rebuild failed");
            }
            session.reload_gtk()?;
            println!(
                "Archcar PID {} unchanged | GTK PID {} -> {}",
                archcar_pid,
                previous_gtk_pid,
                session.gtk_pid()
            );
            Ok(false)
        }
        DevAction::Quit => {
            session.shutdown()?;
            Ok(true)
        }
        DevAction::Ignore => Ok(false),
    }
}

fn required_env(name: &str) -> Result<std::ffi::OsString> {
    std::env::var_os(name).ok_or_else(|| anyhow::anyhow!("{name} is not configured"))
}
