#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevKey {
    Character(char),
    Interrupt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevAction {
    ReloadGtk,
    Quit,
    Ignore,
}

pub fn action_for_key(key: DevKey) -> DevAction {
    match key {
        DevKey::Character('r') => DevAction::ReloadGtk,
        DevKey::Character('q') => DevAction::Quit,
        DevKey::Interrupt => DevAction::Quit,
        DevKey::Character(_) => DevAction::Ignore,
    }
}

pub mod process;

pub trait SessionControl {
    fn reload_gtk(&mut self) -> anyhow::Result<()>;
    fn shutdown(&mut self) -> anyhow::Result<()>;
}

impl SessionControl for process::DevSession {
    fn reload_gtk(&mut self) -> anyhow::Result<()> {
        process::DevSession::reload_gtk(self)
    }

    fn shutdown(&mut self) -> anyhow::Result<()> {
        process::DevSession::shutdown(self)
    }
}

pub fn run_actions<S, I>(session: &mut S, actions: I) -> anyhow::Result<()>
where
    S: SessionControl,
    I: IntoIterator<Item = DevAction>,
{
    for action in actions {
        match action {
            DevAction::ReloadGtk => session.reload_gtk()?,
            DevAction::Quit => {
                session.shutdown()?;
                return Ok(());
            }
            DevAction::Ignore => {}
        }
    }
    session.shutdown()
}

#[cfg(test)]
mod runtime_tests {
    use super::{run_actions, DevAction, SessionControl};

    #[derive(Default)]
    struct FakeSession {
        reloads: usize,
        shutdowns: usize,
    }

    impl SessionControl for FakeSession {
        fn reload_gtk(&mut self) -> anyhow::Result<()> {
            self.reloads += 1;
            Ok(())
        }

        fn shutdown(&mut self) -> anyhow::Result<()> {
            self.shutdowns += 1;
            Ok(())
        }
    }

    #[test]
    fn runtime_reloads_until_quit_then_shuts_down_once() {
        let mut session = FakeSession::default();
        run_actions(
            &mut session,
            [DevAction::Ignore, DevAction::ReloadGtk, DevAction::Quit],
        )
        .unwrap();
        assert_eq!(session.reloads, 1);
        assert_eq!(session.shutdowns, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::{action_for_key, DevAction, DevKey};

    #[test]
    fn maps_flutter_style_keys() {
        assert_eq!(action_for_key(DevKey::Character('r')), DevAction::ReloadGtk);
        assert_eq!(action_for_key(DevKey::Character('q')), DevAction::Quit);
        assert_eq!(action_for_key(DevKey::Interrupt), DevAction::Quit);
        assert_eq!(action_for_key(DevKey::Character('x')), DevAction::Ignore);
    }
}
