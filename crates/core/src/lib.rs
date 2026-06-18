pub mod doctor;
pub mod import;
pub mod mcp;
pub mod paths;
pub mod pty;
pub mod repository;
pub mod settings;
pub mod workspace;

#[cfg(test)]
mod pty_tests {
    use std::time::Duration;

    #[test]
    fn pty_session_accepts_input_and_streams_output() {
        let temp = tempfile::tempdir().unwrap();
        let mut session = crate::pty::PtySession::spawn_shell(temp.path(), Vec::new()).unwrap();

        session.write("printf 'ready:%s\\n' \"$PWD\"\n").unwrap();
        let ready = session
            .read_until("ready:", Duration::from_secs(2))
            .unwrap();
        assert!(ready.contains(temp.path().to_str().unwrap()));

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
}
