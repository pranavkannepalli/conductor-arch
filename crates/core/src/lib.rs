pub mod agent_tools;
pub mod archcar;
pub mod chat_attachments;
pub mod chat_store;
pub mod codex_tui;
pub mod doctor;
pub mod env_flags;
pub mod git_review_service;
pub mod github_pr;
pub mod harness;
pub mod import;
pub mod linear;
pub mod local_chat;
pub mod mcp;
pub mod model_registry;
pub mod paths;
pub mod platform;
pub mod provider_adapters;
pub mod provider_capabilities;
pub mod provider_events;
pub mod provider_inputs;
pub mod provider_interactions;
pub mod provider_projection;
pub mod pty;
pub mod redaction;
pub mod repository;
pub mod runtime_session_store;
pub mod session_event;
pub mod session_pipeline;
pub mod session_state;
pub mod settings;
pub mod storage;
pub mod terminal_logs;
pub mod todos;
pub mod workflow_actions;
pub mod workspace;

#[cfg(test)]
mod pty_tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn pty_session_accepts_input_and_streams_output() {
        let temp = tempfile::tempdir().unwrap();
        let marker = "archductor-pty-ready";
        let mut session = crate::pty::PtySession::spawn(
            PathBuf::from("/bin/sh"),
            Vec::new(),
            temp.path(),
            vec![("LC_PTY_TEST_MARKER".to_owned(), OsString::from(marker))],
            24,
            80,
        )
        .unwrap();

        session
            .write("printf 'ready:%s\\n%s\\n' \"$PWD\" \"$LC_PTY_TEST_MARKER\"\n")
            .unwrap();
        let ready = session.read_until(marker, Duration::from_secs(2)).unwrap();
        assert!(ready.contains(temp.path().canonicalize().unwrap().to_str().unwrap()));

        session
            .write("read line; printf 'echo:%s\\n' \"$line\"; exit\n")
            .unwrap();
        session.write("from-pty\n").unwrap();
        let echoed = session
            .read_until("echo:from-pty", Duration::from_secs(2))
            .unwrap();

        assert!(echoed.contains("echo:from-pty"));
        session.stop().unwrap();
    }

    #[test]
    fn pty_session_resize_updates_child_terminal_size() {
        let temp = tempfile::tempdir().unwrap();
        let mut session = crate::pty::PtySession::spawn_shell(temp.path(), Vec::new()).unwrap();

        session.resize(33, 111).unwrap();
        session.write("stty size; exit\n").unwrap();
        let size = session
            .read_until("33 111", Duration::from_secs(2))
            .unwrap();

        assert!(size.contains("33 111"));
        session.stop().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pty_stop_sends_sigterm_before_force_kill() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("term.marker");
        let script = format!(
            "trap 'echo term > {}; exit 0' TERM; while :; do sleep 1; done",
            marker.display()
        );
        let mut session = crate::pty::PtySession::spawn(
            PathBuf::from("/bin/sh"),
            vec!["-c".to_owned(), script],
            temp.path(),
            Vec::new(),
            24,
            80,
        )
        .unwrap();

        std::thread::sleep(Duration::from_millis(100));
        session.stop().unwrap();

        assert_eq!(std::fs::read_to_string(marker).unwrap(), "term\n");
    }
}
