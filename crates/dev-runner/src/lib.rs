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
    use super::{reload_gtk_after_build, run_actions, DevAction, GtkReloadControl, SessionControl};

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

    #[test]
    fn reload_stops_locked_gtk_before_build_and_restarts_afterward() {
        struct ReloadSession(std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>);
        impl GtkReloadControl for ReloadSession {
            fn stop_gtk(&mut self) -> anyhow::Result<()> {
                self.0.borrow_mut().push("stop");
                Ok(())
            }
            fn start_gtk(&mut self) -> anyhow::Result<()> {
                self.0.borrow_mut().push("start");
                Ok(())
            }
        }

        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut session = ReloadSession(events.clone());
        reload_gtk_after_build(&mut session, || {
            events.borrow_mut().push("build");
            Ok(())
        })
        .unwrap();

        assert_eq!(*events.borrow(), ["stop", "build", "start"]);
    }

    #[test]
    fn reload_reports_build_and_restart_failures_together() {
        struct FailingRestart;
        impl GtkReloadControl for FailingRestart {
            fn stop_gtk(&mut self) -> anyhow::Result<()> {
                Ok(())
            }
            fn start_gtk(&mut self) -> anyhow::Result<()> {
                anyhow::bail!("restart exploded")
            }
        }

        let error = reload_gtk_after_build(&mut FailingRestart, || anyhow::bail!("build exploded"))
            .unwrap_err();

        assert_eq!(
            format!("{error:#}"),
            "GTK build failed (build exploded) and GTK restart failed (restart exploded)"
        );
    }
}

pub trait GtkReloadControl {
    fn stop_gtk(&mut self) -> anyhow::Result<()>;
    fn start_gtk(&mut self) -> anyhow::Result<()>;
}

impl GtkReloadControl for process::DevSession {
    fn stop_gtk(&mut self) -> anyhow::Result<()> {
        process::DevSession::stop_gtk(self)
    }

    fn start_gtk(&mut self) -> anyhow::Result<()> {
        process::DevSession::start_gtk(self)
    }
}

pub fn reload_gtk_after_build<S, B>(session: &mut S, build: B) -> anyhow::Result<()>
where
    S: GtkReloadControl,
    B: FnOnce() -> anyhow::Result<()>,
{
    session.stop_gtk()?;
    if let Err(error) = build() {
        return match session.start_gtk() {
            Ok(()) => Err(error),
            Err(restart_error) => Err(anyhow::anyhow!(
                "GTK build failed ({error:#}) and GTK restart failed ({restart_error:#})"
            )),
        };
    }
    session.start_gtk()
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
