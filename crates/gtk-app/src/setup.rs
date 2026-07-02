use adw::ApplicationWindow;
use gtk::prelude::*;
use gtk::{Box as GBox, Label, LinkButton, Orientation};
use linux_archductor_core::doctor::{setup_blockers, SetupBlocker, SetupReadiness};

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
        "Linux Archductor needs GitHub CLI plus at least one local agent before workspace features can run.",
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
            let readiness = SetupReadiness::from_host();
            render_setup_status(&status_list, &readiness);
            let blockers = setup_blockers(&readiness);
            if blockers.is_empty() {
                dialog.close();
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
        "Required for GitHub projects, PRs, and checks",
        readiness.gh_installed,
        true,
    ));
    container.append(&setup_status_row(
        "Codex",
        "Agent option",
        readiness.codex_installed,
        false,
    ));
    container.append(&setup_status_row(
        "Claude",
        "Agent option",
        readiness.claude_installed,
        false,
    ));
    container.append(&setup_status_row(
        "OpenCode",
        "Agent option",
        readiness.opencode_installed,
        false,
    ));
}

fn setup_status_row(name: &str, detail: &str, installed: bool, required: bool) -> GBox {
    let row = GBox::new(Orientation::Horizontal, 10);
    row.add_css_class("setup-status-row");
    row.add_css_class(if installed {
        "setup-status-ready"
    } else if required {
        "setup-status-missing-required"
    } else {
        "setup-status-missing"
    });

    let state = Label::new(Some(if installed { "Ready" } else { "Missing" }));
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
        [SetupBlocker::MissingGithubCli] => "Install GitHub CLI, then press Recheck.".to_owned(),
        [SetupBlocker::MissingAgent] => {
            "Install Codex, Claude, or OpenCode, then press Recheck.".to_owned()
        }
        _ => "Install GitHub CLI and at least one agent, then press Recheck.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_feedback_summarizes_missing_github_cli() {
        let readiness = SetupReadiness {
            gh_installed: false,
            codex_installed: true,
            claude_installed: false,
            opencode_installed: false,
        };

        assert_eq!(
            setup_feedback(&readiness),
            "Install GitHub CLI, then press Recheck."
        );
    }

    #[test]
    fn setup_feedback_summarizes_missing_agent() {
        let readiness = SetupReadiness {
            gh_installed: true,
            codex_installed: false,
            claude_installed: false,
            opencode_installed: false,
        };

        assert_eq!(
            setup_feedback(&readiness),
            "Install Codex, Claude, or OpenCode, then press Recheck."
        );
    }
}
