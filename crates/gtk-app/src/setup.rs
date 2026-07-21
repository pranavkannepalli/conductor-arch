use archductor_core::doctor::{
    refresh_process_environment, setup_blockers, SetupBlocker, SetupCheck, SetupReadiness,
};
use gtk::prelude::*;
use gtk::{ApplicationWindow, Box as GBox, Label, LinkButton, Orientation};

use crate::buttons::text_button;

pub(crate) fn show_blocking_setup_if_needed(parent: &ApplicationWindow) {
    let readiness = SetupReadiness::from_host();
    if setup_blockers(&readiness).is_empty() {
        return;
    }

    let dialog = gtk::Window::builder()
        .title("Setup required")
        .transient_for(parent)
        .modal(true)
        .default_width(560)
        .default_height(520)
        .build();
    dialog.set_deletable(false);

    let body = GBox::new(Orientation::Vertical, 14);
    body.add_css_class("setup-modal");
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);

    let title = Label::new(Some("Finish setup"));
    title.add_css_class("setup-title");
    title.set_xalign(0.0);
    body.append(&title);

    let copy = Label::new(Some(
        "Archductor needs GitHub CLI plus a signed-in Codex or Claude CLI before chat features can run.",
    ));
    copy.add_css_class("setup-copy");
    copy.set_wrap(true);
    copy.set_xalign(0.0);
    body.append(&copy);

    let status_list = GBox::new(Orientation::Vertical, 8);
    status_list.add_css_class("setup-status-list");
    body.append(&status_list);

    let guidance = GBox::new(Orientation::Vertical, 8);
    guidance.add_css_class("setup-guidance");
    guidance.append(&setup_link("Install GitHub CLI", "https://cli.github.com/"));
    guidance.append(&setup_link(
        "Install Codex",
        "https://developers.openai.com/codex/cli",
    ));
    guidance.append(&setup_link(
        "Install Claude Code",
        "https://docs.anthropic.com/en/docs/claude-code",
    ));
    guidance.append(&setup_link("Install OpenCode", "https://opencode.ai/"));
    body.append(&guidance);

    let feedback = Label::new(None);
    feedback.add_css_class("setup-feedback");
    feedback.set_xalign(0.0);
    feedback.set_wrap(true);
    body.append(&feedback);

    let actions = GBox::new(Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let recheck = text_button("Recheck");
    recheck.add_css_class("suggested-action");
    actions.append(&recheck);
    body.append(&actions);

    render_setup_status(&status_list, &readiness);
    feedback.set_text(&setup_feedback(&readiness));

    {
        let dialog = dialog.clone();
        let status_list = status_list.clone();
        let feedback = feedback.clone();
        recheck.connect_clicked(move |_| {
            let refresh_error = refresh_process_environment().err();
            let readiness = SetupReadiness::from_host();
            render_setup_status(&status_list, &readiness);
            let blockers = setup_blockers(&readiness);
            if blockers.is_empty() {
                dialog.close();
            } else if let Some(error) = refresh_error {
                feedback.set_text(&format!(
                    "{error} Restart Archductor if the tool was just installed.\n{}",
                    setup_feedback(&readiness)
                ));
            } else {
                feedback.set_text(&setup_feedback(&readiness));
            }
        });
    }

    dialog.set_child(Some(&body));
    dialog.present();
}

fn setup_link(label: &str, uri: &str) -> LinkButton {
    let link = LinkButton::with_label(uri, label);
    link.add_css_class("setup-link");
    link.set_halign(gtk::Align::Start);
    link
}

fn render_setup_status(container: &GBox, readiness: &SetupReadiness) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    container.append(&setup_status_row(
        "GitHub CLI",
        &readiness.gh.detail,
        &readiness.gh,
        true,
    ));
    container.append(&setup_status_row(
        "Codex",
        &readiness.codex.detail,
        &readiness.codex,
        false,
    ));
    container.append(&setup_status_row(
        "Claude",
        &readiness.claude.detail,
        &readiness.claude,
        false,
    ));
    container.append(&setup_status_row(
        "OpenCode",
        &readiness.opencode.detail,
        &readiness.opencode,
        false,
    ));
    container.append(&setup_status_row(
        "Selected provider",
        &selected_provider_detail(readiness),
        &selected_provider_check(readiness),
        true,
    ));
}

fn setup_status_row(name: &str, detail: &str, check: &SetupCheck, required: bool) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 10);
    row.add_css_class("setup-status-row");
    row.add_css_class(if check.ready {
        "setup-status-ready"
    } else if required {
        "setup-status-missing-required"
    } else {
        "setup-status-missing"
    });

    let state = Label::new(Some(if check.ready {
        "Ready"
    } else if check.installed {
        "Action"
    } else {
        "Missing"
    }));
    state.add_css_class("setup-status-pill");
    row.append(&state);

    let text = GBox::new(Orientation::Vertical, 2);
    text.set_hexpand(true);
    let name_label = Label::new(Some(name));
    name_label.add_css_class("setup-status-name");
    name_label.set_xalign(0.0);
    let detail_label = Label::new(Some(detail));
    detail_label.add_css_class("setup-status-detail");
    detail_label.set_xalign(0.0);
    detail_label.set_wrap(true);
    text.append(&name_label);
    text.append(&detail_label);
    row.append(&text);

    row
}

fn setup_feedback(readiness: &SetupReadiness) -> String {
    match setup_blockers(readiness).as_slice() {
        [] => "Setup is complete.".to_owned(),
        [SetupBlocker::GithubUnavailable] if readiness.gh.installed => {
            "Authenticate GitHub CLI, then press Recheck.".to_owned()
        }
        [SetupBlocker::GithubUnavailable] => {
            "Install and authenticate GitHub CLI, then press Recheck.".to_owned()
        }
        [SetupBlocker::MissingAgent] if readiness.codex.installed || readiness.claude.installed => {
            "Sign in to Codex or Claude, then press Recheck.".to_owned()
        }
        [SetupBlocker::MissingAgent] => {
            "Install and sign in to Codex or Claude, then press Recheck.".to_owned()
        }
        [SetupBlocker::SelectedProviderUnavailable] => {
            "Choose a ready provider or sign in to the selected provider, then press Recheck."
                .to_owned()
        }
        _ => {
            "Install or authenticate GitHub CLI and Codex or Claude, then press Recheck.".to_owned()
        }
    }
}

fn selected_provider_check(readiness: &SetupReadiness) -> SetupCheck {
    match readiness.first_ready_launchable_provider() {
        Some(provider) => SetupCheck::ready(format!("{provider} will be selected for new chats.")),
        None if readiness.opencode.ready => SetupCheck::blocked(
            "OpenCode is ready, but this build cannot launch OpenCode chat sessions yet.",
        ),
        None => SetupCheck::missing("No launchable chat provider is ready."),
    }
}

fn selected_provider_detail(readiness: &SetupReadiness) -> String {
    selected_provider_check(readiness).detail
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_recheck_refreshes_the_host_environment_before_probing() {
        let source = include_str!("setup.rs");
        let handler = source
            .split("recheck.connect_clicked")
            .nth(1)
            .expect("recheck handler exists");
        let refresh = handler
            .find("refresh_process_environment")
            .expect("recheck refreshes the process environment");
        let probe = handler
            .find("SetupReadiness::from_host")
            .expect("recheck probes setup readiness");

        assert!(
            refresh < probe,
            "environment refresh must happen before probes"
        );
    }

    #[test]
    fn setup_feedback_summarizes_missing_github_cli() {
        let readiness = SetupReadiness {
            gh: SetupCheck::missing("missing"),
            codex: SetupCheck::ready("ready"),
            claude: SetupCheck::missing("missing"),
            opencode: SetupCheck::missing("missing"),
        };

        assert_eq!(
            setup_feedback(&readiness),
            "Install and authenticate GitHub CLI, then press Recheck."
        );
    }

    #[test]
    fn setup_feedback_summarizes_missing_agent() {
        let readiness = SetupReadiness {
            gh: SetupCheck::ready("ready"),
            codex: SetupCheck::missing("missing"),
            claude: SetupCheck::missing("missing"),
            opencode: SetupCheck::missing("missing"),
        };

        assert_eq!(
            setup_feedback(&readiness),
            "Install and sign in to Codex or Claude, then press Recheck."
        );
    }

    #[test]
    fn setup_feedback_summarizes_installed_but_blocked_agent() {
        let readiness = SetupReadiness {
            gh: SetupCheck::ready("ready"),
            codex: SetupCheck::blocked("blocked"),
            claude: SetupCheck::missing("missing"),
            opencode: SetupCheck::ready("ready"),
        };

        assert_eq!(
            setup_feedback(&readiness),
            "Sign in to Codex or Claude, then press Recheck."
        );
    }

    #[test]
    fn selected_provider_prefers_ready_launchable_agent() {
        let readiness = SetupReadiness {
            gh: SetupCheck::ready("ready"),
            codex: SetupCheck::missing("missing"),
            claude: SetupCheck::ready("ready"),
            opencode: SetupCheck::ready("ready"),
        };

        assert_eq!(
            selected_provider_detail(&readiness),
            "claude will be selected for new chats."
        );
        assert!(
            archductor_core::doctor::setup_blockers_for_provider(&readiness, Some("claude"))
                .is_empty()
        );
    }
}
