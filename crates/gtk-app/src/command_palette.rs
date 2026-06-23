use crate::state::{AppPage, WorkspaceTab};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Keybindings {
    pub command_palette: KeyShortcut,
    pub refresh: KeyShortcut,
    pub toggle_sidebar: KeyShortcut,
    pub tab_changes: Option<KeyShortcut>,
    pub tab_checks: Option<KeyShortcut>,
    pub tab_review: Option<KeyShortcut>,
    pub tab_chat: Option<KeyShortcut>,
    pub tab_terminal: Option<KeyShortcut>,
    pub tab_todos: Option<KeyShortcut>,
    pub tab_processes: Option<KeyShortcut>,
    pub tab_checkpoints: Option<KeyShortcut>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyShortcut {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    pub key: char,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShortcutAction {
    CommandPalette,
    Refresh,
    ToggleSidebar,
    NavigateTab(WorkspaceTab),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PaletteTarget {
    Page(AppPage),
    WorkspaceTab(WorkspaceTab),
    Refresh,
    ToggleSidebar,
    RunCommand(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaletteCommand {
    pub label: String,
    pub shortcut: Option<String>,
    pub target: PaletteTarget,
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            command_palette: KeyShortcut::new(true, false, false, false, 'k'),
            refresh: KeyShortcut::new(true, false, false, false, 'r'),
            toggle_sidebar: KeyShortcut::new(true, false, false, false, 'b'),
            tab_changes: None,
            tab_checks: None,
            tab_review: None,
            tab_chat: None,
            tab_terminal: None,
            tab_todos: None,
            tab_processes: None,
            tab_checkpoints: None,
        }
    }
}

impl Keybindings {
    pub(crate) fn from_config(value: Option<&str>) -> Self {
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            return Self::default();
        };
        match normalize_palette_query(value).as_str() {
            "default" | "conductor" | "native" => return Self::default(),
            "vim" => {
                return Self {
                    command_palette: KeyShortcut::new(true, false, false, false, 'p'),
                    ..Self::default()
                };
            }
            _ => {}
        }

        let mut bindings = Self::default();
        for entry in value.split([',', ';', '\n']) {
            let Some((name, shortcut)) = entry.split_once('=') else {
                continue;
            };
            let Some(shortcut) = KeyShortcut::parse(shortcut) else {
                continue;
            };
            match normalize_palette_query(name).as_str() {
                "palette" | "commandpalette" | "commands" => bindings.command_palette = shortcut,
                "refresh" | "reload" => bindings.refresh = shortcut,
                "sidebar" | "togglesidebar" | "nav" => bindings.toggle_sidebar = shortcut,
                "changes" | "changestab" | "diff" => bindings.tab_changes = Some(shortcut),
                "checks" | "checkstab" | "ci" | "pr" => bindings.tab_checks = Some(shortcut),
                "review" | "reviewtab" => bindings.tab_review = Some(shortcut),
                "chat" | "chatterminal" | "chattab" | "agents" => {
                    bindings.tab_chat = Some(shortcut)
                }
                "terminal" | "bigterminal" | "terminaltab" => {
                    bindings.tab_terminal = Some(shortcut)
                }
                "todos" | "todostab" | "tasks" => bindings.tab_todos = Some(shortcut),
                "processes" | "processestab" | "runs" => bindings.tab_processes = Some(shortcut),
                "checkpoints" | "checkpointstab" | "restore" => {
                    bindings.tab_checkpoints = Some(shortcut)
                }
                _ => {}
            }
        }
        bindings
    }

    pub(crate) fn shortcut_for_target(&self, target: &PaletteTarget) -> Option<String> {
        match target {
            PaletteTarget::Refresh => Some(self.refresh.display()),
            PaletteTarget::ToggleSidebar => Some(self.toggle_sidebar.display()),
            PaletteTarget::WorkspaceTab(WorkspaceTab::Changes) => {
                self.tab_changes.as_ref().map(|s| s.display())
            }
            PaletteTarget::WorkspaceTab(WorkspaceTab::Checks) => {
                self.tab_checks.as_ref().map(|s| s.display())
            }
            PaletteTarget::WorkspaceTab(WorkspaceTab::Review) => {
                self.tab_review.as_ref().map(|s| s.display())
            }
            PaletteTarget::WorkspaceTab(WorkspaceTab::Chats) => {
                self.tab_chat.as_ref().map(|s| s.display())
            }
            PaletteTarget::WorkspaceTab(WorkspaceTab::Terminal) => {
                self.tab_terminal.as_ref().map(|s| s.display())
            }
            PaletteTarget::WorkspaceTab(WorkspaceTab::Todos) => {
                self.tab_todos.as_ref().map(|s| s.display())
            }
            PaletteTarget::WorkspaceTab(WorkspaceTab::Processes) => {
                self.tab_processes.as_ref().map(|s| s.display())
            }
            PaletteTarget::WorkspaceTab(WorkspaceTab::Checkpoints) => {
                self.tab_checkpoints.as_ref().map(|s| s.display())
            }
            _ => None,
        }
    }

    pub(crate) fn action_for_event(
        &self,
        key: char,
        ctrl: bool,
        alt: bool,
        shift: bool,
        meta: bool,
    ) -> Option<ShortcutAction> {
        let event = KeyShortcut::new(ctrl, alt, shift, meta, key);
        if event == self.command_palette {
            return Some(ShortcutAction::CommandPalette);
        } else if event == self.refresh {
            return Some(ShortcutAction::Refresh);
        } else if event == self.toggle_sidebar {
            return Some(ShortcutAction::ToggleSidebar);
        }
        let tab_pairs: &[(_, WorkspaceTab)] = &[
            (self.tab_changes.as_ref(), WorkspaceTab::Changes),
            (self.tab_checks.as_ref(), WorkspaceTab::Checks),
            (self.tab_review.as_ref(), WorkspaceTab::Review),
            (self.tab_chat.as_ref(), WorkspaceTab::Chats),
            (self.tab_terminal.as_ref(), WorkspaceTab::Terminal),
            (self.tab_todos.as_ref(), WorkspaceTab::Todos),
            (self.tab_processes.as_ref(), WorkspaceTab::Processes),
            (self.tab_checkpoints.as_ref(), WorkspaceTab::Checkpoints),
        ];
        for (shortcut_opt, tab) in tab_pairs {
            if let Some(shortcut) = shortcut_opt {
                if event == **shortcut {
                    return Some(ShortcutAction::NavigateTab(tab.clone()));
                }
            }
        }
        None
    }
}

impl KeyShortcut {
    fn new(ctrl: bool, alt: bool, shift: bool, meta: bool, key: char) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            meta,
            key: key.to_ascii_lowercase(),
        }
    }

    fn parse(value: &str) -> Option<Self> {
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut meta = false;
        let mut key = None;
        for part in value
            .split('+')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            match normalize_palette_query(part).as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" | "option" => alt = true,
                "shift" => shift = true,
                "meta" | "cmd" | "command" | "super" => meta = true,
                token if token.chars().count() == 1 => key = token.chars().next(),
                _ => return None,
            }
        }
        key.map(|key| Self::new(ctrl, alt, shift, meta, key))
    }

    fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_owned());
        }
        if self.alt {
            parts.push("Alt".to_owned());
        }
        if self.shift {
            parts.push("Shift".to_owned());
        }
        if self.meta {
            parts.push("Meta".to_owned());
        }
        parts.push(self.key.to_ascii_uppercase().to_string());
        parts.join("+")
    }
}

pub(crate) fn palette_commands(
    has_workspace: bool,
    keybindings: &Keybindings,
    custom_commands: &[String],
) -> Vec<PaletteCommand> {
    let cmd = |label: &'static str, target: PaletteTarget| PaletteCommand {
        label: label.to_owned(),
        shortcut: None,
        target,
    };
    let mut commands = vec![
        cmd("Dashboard", PaletteTarget::Page(AppPage::Dashboard)),
        cmd("Settings", PaletteTarget::Page(AppPage::Settings)),
        cmd("History", PaletteTarget::Page(AppPage::History)),
        cmd("Refresh", PaletteTarget::Refresh),
        cmd("Toggle Sidebar", PaletteTarget::ToggleSidebar),
    ];

    if has_workspace {
        commands.extend([
            cmd("Workspace", PaletteTarget::Page(AppPage::Workspace)),
            cmd(
                "Changes",
                PaletteTarget::WorkspaceTab(WorkspaceTab::Changes),
            ),
            cmd("Checks", PaletteTarget::WorkspaceTab(WorkspaceTab::Checks)),
            cmd("Review", PaletteTarget::WorkspaceTab(WorkspaceTab::Review)),
            cmd(
                "Chat / Terminal",
                PaletteTarget::WorkspaceTab(WorkspaceTab::Chats),
            ),
            cmd(
                "Big Terminal",
                PaletteTarget::WorkspaceTab(WorkspaceTab::Terminal),
            ),
            cmd("Todos", PaletteTarget::WorkspaceTab(WorkspaceTab::Todos)),
            cmd(
                "Processes",
                PaletteTarget::WorkspaceTab(WorkspaceTab::Processes),
            ),
            cmd(
                "Checkpoints",
                PaletteTarget::WorkspaceTab(WorkspaceTab::Checkpoints),
            ),
        ]);
        for entry in custom_commands {
            if let Some(custom) = custom_palette_command_from_config(entry) {
                commands.push(custom);
            }
        }
    }

    commands
        .into_iter()
        .map(|mut command| {
            command.shortcut = keybindings.shortcut_for_target(&command.target);
            command
        })
        .collect()
}

fn custom_palette_command_from_config(entry: &str) -> Option<PaletteCommand> {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = normalize_palette_query(trimmed);
    let (label, command) = match normalized.as_str() {
        "test" => ("Run Tests", "pnpm test"),
        "lint" => ("Run Lint", "pnpm lint"),
        "build" => ("Build", "pnpm build"),
        "typecheck" | "type" => ("Typecheck", "pnpm typecheck"),
        "ci" => ("CI", "pnpm test && pnpm lint && pnpm build"),
        "status" | "gitstatus" => ("Git Status", "git status --short --branch"),
        "diff" | "gitdiff" => ("Git Diff", "git diff --stat && git diff -- ."),
        "env" => ("Env", "env | sort | grep '^CONDUCTOR_'"),
        "files" => (
            "Files",
            "find . -maxdepth 2 -type f | sort | sed 's#^./##' | head -80",
        ),
        _ => {
            let (lbl, cmd) = trimmed
                .split_once('=')
                .or_else(|| trimmed.split_once(": "))
                .or_else(|| trimmed.split_once(':'))?;
            let lbl = lbl.trim();
            let cmd = cmd.trim();
            if lbl.is_empty() || cmd.is_empty() {
                return None;
            }
            return Some(PaletteCommand {
                label: lbl.to_owned(),
                shortcut: None,
                target: PaletteTarget::RunCommand(cmd.to_owned()),
            });
        }
    };
    Some(PaletteCommand {
        label: label.to_owned(),
        shortcut: None,
        target: PaletteTarget::RunCommand(command.to_owned()),
    })
}

pub(crate) fn filter_palette_commands<'a>(
    commands: &'a [PaletteCommand],
    query: &str,
) -> Vec<&'a PaletteCommand> {
    let query = normalize_palette_query(query);
    if query.is_empty() {
        return commands.iter().collect();
    }
    commands
        .iter()
        .filter(|command| palette_command_matches(command, &query))
        .collect()
}

fn palette_command_matches(command: &PaletteCommand, normalized_query: &str) -> bool {
    palette_command_search_terms(command)
        .iter()
        .any(|term| normalize_palette_query(term).contains(normalized_query))
}

fn palette_command_search_terms(command: &PaletteCommand) -> Vec<String> {
    let mut terms = vec![command.label.to_owned()];
    if let Some(shortcut) = &command.shortcut {
        terms.push(shortcut.clone());
    }
    terms.extend(match &command.target {
        PaletteTarget::Page(AppPage::Dashboard) => vec!["home".to_owned(), "overview".to_owned()],
        PaletteTarget::Page(AppPage::Projects) => Vec::new(),
        PaletteTarget::Page(AppPage::History) => vec!["archive".to_owned(), "past".to_owned()],
        PaletteTarget::Page(AppPage::Workspace) => vec!["worktree".to_owned(), "branch".to_owned()],
        PaletteTarget::Page(AppPage::Settings) => vec!["config".to_owned()],
        PaletteTarget::Page(AppPage::Review) => vec!["review".to_owned()],
        PaletteTarget::WorkspaceTab(WorkspaceTab::Chats) => {
            vec!["chat".to_owned(), "agent".to_owned(), "session".to_owned()]
        }
        PaletteTarget::WorkspaceTab(WorkspaceTab::Changes) => {
            vec!["diff".to_owned(), "files".to_owned()]
        }
        PaletteTarget::WorkspaceTab(WorkspaceTab::Review) => {
            vec![
                "review".to_owned(),
                "comments".to_owned(),
                "localreview".to_owned(),
            ]
        }
        PaletteTarget::WorkspaceTab(WorkspaceTab::Checks) => {
            vec!["ci".to_owned(), "pr".to_owned(), "github".to_owned()]
        }
        PaletteTarget::WorkspaceTab(WorkspaceTab::Checkpoints) => {
            vec!["checkpoint".to_owned(), "restore".to_owned()]
        }
        PaletteTarget::WorkspaceTab(WorkspaceTab::Todos) => {
            vec!["todo".to_owned(), "tasks".to_owned()]
        }
        PaletteTarget::WorkspaceTab(WorkspaceTab::Processes) => {
            vec!["process".to_owned(), "runs".to_owned()]
        }
        PaletteTarget::WorkspaceTab(WorkspaceTab::Terminal) => {
            vec!["terminal".to_owned(), "shell".to_owned(), "big".to_owned()]
        }
        PaletteTarget::Refresh => vec!["reload".to_owned(), "sync".to_owned()],
        PaletteTarget::ToggleSidebar => vec!["sidebar".to_owned(), "nav".to_owned()],
        PaletteTarget::RunCommand(cmd) => vec![cmd.clone()],
    });
    terms
}

fn normalize_palette_query(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_commands_include_global_navigation_and_shortcuts() {
        let keybindings = Keybindings::default();
        let commands = palette_commands(false, &keybindings, &[]);

        assert!(commands.iter().any(|command| command.label == "Dashboard"
            && command.target == PaletteTarget::Page(AppPage::Dashboard)));
        assert!(commands
            .iter()
            .any(|command| command.label == "Refresh"
                && command.shortcut.as_deref() == Some("Ctrl+R")));
        assert!(commands
            .iter()
            .any(|command| command.label == "Toggle Sidebar"
                && command.shortcut.as_deref() == Some("Ctrl+B")));
        assert!(!commands
            .iter()
            .any(|command| command.label == "Big Terminal"));
    }

    #[test]
    fn palette_commands_include_workspace_tabs_when_workspace_selected() {
        let keybindings = Keybindings::default();
        let commands = palette_commands(true, &keybindings, &[]);

        assert!(commands
            .iter()
            .any(|command| command.label == "Big Terminal"
                && command.target == PaletteTarget::WorkspaceTab(WorkspaceTab::Terminal)));
        assert!(commands.iter().any(|command| command.label == "Changes"
            && command.target == PaletteTarget::WorkspaceTab(WorkspaceTab::Changes)));
    }

    #[test]
    fn palette_filter_matches_label_shortcut_and_aliases() {
        let keybindings = Keybindings::default();
        let commands = palette_commands(true, &keybindings, &[]);

        let terminal = filter_palette_commands(&commands, "term");
        assert_eq!(terminal[0].label, "Chat / Terminal");
        assert!(terminal
            .iter()
            .any(|command| command.label == "Big Terminal"));

        let refresh = filter_palette_commands(&commands, "ctrl+r");
        assert_eq!(refresh.len(), 1);
        assert_eq!(refresh[0].label, "Refresh");

        let checks = filter_palette_commands(&commands, "ci");
        assert!(checks.iter().any(|command| command.label == "Checks"));

        let chat = filter_palette_commands(&commands, "chat");
        assert_eq!(
            chat[0].target,
            PaletteTarget::WorkspaceTab(WorkspaceTab::Chats)
        );
    }

    #[test]
    fn palette_filter_hides_workspace_commands_without_workspace() {
        let keybindings = Keybindings::default();
        let commands = palette_commands(false, &keybindings, &[]);

        assert!(filter_palette_commands(&commands, "terminal").is_empty());
        assert!(filter_palette_commands(&commands, "project").is_empty());
    }

    #[test]
    fn keybindings_parse_presets_and_custom_entries() {
        let vim = Keybindings::from_config(Some("vim"));
        assert_eq!(vim.command_palette.display(), "Ctrl+P");
        assert_eq!(vim.refresh.display(), "Ctrl+R");

        let custom = Keybindings::from_config(Some(
            "palette=ctrl+p, refresh=ctrl+shift+r, sidebar=ctrl+alt+b",
        ));
        assert_eq!(custom.command_palette.display(), "Ctrl+P");
        assert_eq!(custom.refresh.display(), "Ctrl+Shift+R");
        assert_eq!(custom.toggle_sidebar.display(), "Ctrl+Alt+B");
    }

    #[test]
    fn keybindings_match_configured_events() {
        let custom = Keybindings::from_config(Some(
            "palette=ctrl+p, refresh=ctrl+shift+r, sidebar=ctrl+alt+b",
        ));

        assert_eq!(
            custom.action_for_event('p', true, false, false, false),
            Some(ShortcutAction::CommandPalette)
        );
        assert_eq!(
            custom.action_for_event('r', true, false, true, false),
            Some(ShortcutAction::Refresh)
        );
        assert_eq!(
            custom.action_for_event('b', true, true, false, false),
            Some(ShortcutAction::ToggleSidebar)
        );
        assert_eq!(
            custom.action_for_event('r', true, false, false, false),
            None
        );
    }

    #[test]
    fn keybindings_parse_tab_shortcuts() {
        let bindings =
            Keybindings::from_config(Some("changes=ctrl+1, checks=ctrl+2, terminal=ctrl+5"));
        assert_eq!(
            bindings
                .tab_changes
                .as_ref()
                .map(|s| s.display())
                .as_deref(),
            Some("Ctrl+1")
        );
        assert_eq!(
            bindings.tab_checks.as_ref().map(|s| s.display()).as_deref(),
            Some("Ctrl+2")
        );
        assert_eq!(
            bindings
                .tab_terminal
                .as_ref()
                .map(|s| s.display())
                .as_deref(),
            Some("Ctrl+5")
        );
        assert!(bindings.tab_todos.is_none());
    }

    #[test]
    fn keybindings_navigate_tab_action() {
        let bindings = Keybindings::from_config(Some("changes=ctrl+1, todos=ctrl+6"));
        assert_eq!(
            bindings.action_for_event('1', true, false, false, false),
            Some(ShortcutAction::NavigateTab(WorkspaceTab::Changes))
        );
        assert_eq!(
            bindings.action_for_event('6', true, false, false, false),
            Some(ShortcutAction::NavigateTab(WorkspaceTab::Todos))
        );
        assert_eq!(
            bindings.action_for_event('9', true, false, false, false),
            None
        );
    }

    #[test]
    fn palette_commands_include_custom_commands() {
        let keybindings = Keybindings::default();
        let custom = vec![
            "test".to_owned(),
            "Open Docs=xdg-open https://example.com".to_owned(),
        ];
        let commands = palette_commands(true, &keybindings, &custom);
        assert!(commands.iter().any(|c| c.label == "Run Tests"
            && c.target == PaletteTarget::RunCommand("pnpm test".to_owned())));
        assert!(commands.iter().any(|c| c.label == "Open Docs"
            && c.target == PaletteTarget::RunCommand("xdg-open https://example.com".to_owned())));
    }

    #[test]
    fn palette_commands_custom_commands_hidden_without_workspace() {
        let keybindings = Keybindings::default();
        let custom = vec!["test".to_owned()];
        let commands = palette_commands(false, &keybindings, &custom);
        assert!(!commands.iter().any(|c| c.label == "Run Tests"));
    }

    #[test]
    fn tab_shortcuts_shown_in_palette() {
        let bindings = Keybindings::from_config(Some("changes=ctrl+1"));
        let commands = palette_commands(true, &bindings, &[]);
        let changes = commands
            .iter()
            .find(|c| c.target == PaletteTarget::WorkspaceTab(WorkspaceTab::Changes))
            .unwrap();
        assert_eq!(changes.shortcut.as_deref(), Some("Ctrl+1"));
    }
}
