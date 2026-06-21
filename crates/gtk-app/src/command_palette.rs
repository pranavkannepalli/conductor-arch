use crate::state::{AppPage, WorkspaceTab};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PaletteTarget {
    Page(AppPage),
    WorkspaceTab(WorkspaceTab),
    Refresh,
    ToggleSidebar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaletteCommand {
    pub label: &'static str,
    pub shortcut: Option<&'static str>,
    pub target: PaletteTarget,
}

pub(crate) fn palette_commands(has_workspace: bool) -> Vec<PaletteCommand> {
    let mut commands = vec![
        PaletteCommand {
            label: "Dashboard",
            shortcut: None,
            target: PaletteTarget::Page(AppPage::Dashboard),
        },
        PaletteCommand {
            label: "Projects",
            shortcut: None,
            target: PaletteTarget::Page(AppPage::Projects),
        },
        PaletteCommand {
            label: "History",
            shortcut: None,
            target: PaletteTarget::Page(AppPage::History),
        },
        PaletteCommand {
            label: "Refresh",
            shortcut: Some("Ctrl+R"),
            target: PaletteTarget::Refresh,
        },
        PaletteCommand {
            label: "Toggle Sidebar",
            shortcut: Some("Ctrl+B"),
            target: PaletteTarget::ToggleSidebar,
        },
    ];

    if has_workspace {
        commands.extend([
            PaletteCommand {
                label: "Workspace",
                shortcut: None,
                target: PaletteTarget::Page(AppPage::Workspace),
            },
            PaletteCommand {
                label: "Changes",
                shortcut: None,
                target: PaletteTarget::WorkspaceTab(WorkspaceTab::Changes),
            },
            PaletteCommand {
                label: "Chat / Terminal",
                shortcut: None,
                target: PaletteTarget::WorkspaceTab(WorkspaceTab::Chats),
            },
            PaletteCommand {
                label: "Big Terminal",
                shortcut: None,
                target: PaletteTarget::WorkspaceTab(WorkspaceTab::Terminal),
            },
            PaletteCommand {
                label: "Todos",
                shortcut: None,
                target: PaletteTarget::WorkspaceTab(WorkspaceTab::Todos),
            },
            PaletteCommand {
                label: "Processes",
                shortcut: None,
                target: PaletteTarget::WorkspaceTab(WorkspaceTab::Processes),
            },
            PaletteCommand {
                label: "Checkpoints",
                shortcut: None,
                target: PaletteTarget::WorkspaceTab(WorkspaceTab::Checkpoints),
            },
        ]);
    }

    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_commands_include_global_navigation_and_shortcuts() {
        let commands = palette_commands(false);

        assert!(commands.iter().any(|command| command.label == "Dashboard"
            && command.target == PaletteTarget::Page(AppPage::Dashboard)));
        assert!(commands
            .iter()
            .any(|command| command.label == "Refresh" && command.shortcut == Some("Ctrl+R")));
        assert!(
            commands
                .iter()
                .any(|command| command.label == "Toggle Sidebar"
                    && command.shortcut == Some("Ctrl+B"))
        );
        assert!(!commands
            .iter()
            .any(|command| command.label == "Big Terminal"));
    }

    #[test]
    fn palette_commands_include_workspace_tabs_when_workspace_selected() {
        let commands = palette_commands(true);

        assert!(commands
            .iter()
            .any(|command| command.label == "Big Terminal"
                && command.target == PaletteTarget::WorkspaceTab(WorkspaceTab::Terminal)));
        assert!(commands.iter().any(|command| command.label == "Changes"
            && command.target == PaletteTarget::WorkspaceTab(WorkspaceTab::Changes)));
    }
}
