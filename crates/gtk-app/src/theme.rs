pub(crate) fn app_css() -> &'static str {
    APP_CSS
}

const APP_CSS: &str = r#"
@define-color lc-bg #191919;
@define-color lc-surface #1e1e1e;
@define-color lc-surface-raised #202020;
@define-color lc-surface-muted #181818;
@define-color lc-hover #2a2a2a;
@define-color lc-hover-soft #242424;
@define-color lc-border #2a2a2a;
@define-color lc-border-strong #3a3a3a;
@define-color lc-text #e4e4e4;
@define-color lc-text-strong #f8fafc;
@define-color lc-text-muted #8a8a8a;
@define-color lc-success #d0d0d0;
@define-color lc-danger #ff8a8a;

toast {
    background-color: #202020;
    color: @lc-text-strong;
    border: 1px solid #3a3a3a;
    border-radius: 10px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.34);
    margin: 6px;
    padding: 6px;
}

toast > widget {
    margin: 0;
}

toast > button.circular.flat {
    color: #8f8f8f;
    min-width: 28px;
    min-height: 28px;
    padding: 0;
}

toast > button.circular.flat:hover {
    background-color: #303030;
    color: @lc-text-strong;
}

.toast-content {
    min-width: 280px;
    padding: 4px 2px 4px 4px;
}

.toast-icon-shell {
    min-width: 28px;
    min-height: 28px;
    border-radius: 8px;
}

.toast-icon {
    min-width: 16px;
    min-height: 16px;
}

.toast-message {
    color: @lc-text-strong;
    font-size: 13px;
    font-weight: 500;
    line-height: 1.35;
}

.toast-info .toast-icon-shell {
    background-color: rgba(74, 144, 226, 0.16);
    color: #7ab7ff;
}

.toast-success .toast-icon-shell {
    background-color: rgba(54, 179, 126, 0.16);
    color: #64d69b;
}

.toast-warning .toast-icon-shell {
    background-color: rgba(245, 166, 35, 0.16);
    color: #f4bf67;
}

.toast-error .toast-icon-shell {
    background-color: rgba(232, 84, 84, 0.16);
    color: #ff8a8a;
}

window {
    background-color: @lc-bg;
    color: @lc-text;
    font-family: "Mona Sans", "Adwaita Sans", "SF Pro Text", "Segoe UI", "Cantarell", "Noto Sans", sans-serif;
}

button,
entry,
combobox,
textview,
.workspace-card,
.shell-card,
.metric-card,
.kanban-column,
.workspace-row-shell,
.project-row,
.history-row,
.nav-button,
.nav-row,
.ws-tab-shell,
.ws-tab-label,
.ws-tab-close-icon,
.chat-menu-item,
.chat-composer-box,
.chat-inline-event-chip,
.chat-inline-event-body,
.settings-content-panel,
.settings-inspector,
.workspace-modal,
.workspace-modal-preview,
.project-template-card {
    transition-property: background-color, border-color, color, box-shadow, opacity;
    transition-duration: 160ms;
    transition-timing-function: ease-out;
}

.dashboard,
.page-shell,
.history-view {
    background-color: @lc-bg;
    color: @lc-text;
}

.page-header,
.dashboard-header {
    padding: 24px 30px 12px 30px;
    border-bottom: 1px solid #2a2a2a;
    background-color: #1e1e1e;
}

.page-body,
.detail-body,
.page-board,
.kanban-board {
    padding: 24px 30px;
}

.dashboard-title {
    color: #e4e4e4;
    font-size: 22px;
    font-weight: 600;
}

.section-title,
.sidebar-header,
.repo-section-header {
    color: #8a8a8a;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
}

.section-title {
    margin-top: 8px;
}

.workspace-meta,
.card-branch,
.workspace-path-label,
.detail-label {
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
}

.card-meta,
.workspace-meta,
.detail-label,
.project-tab,
.column-count,
.workspace-path-label {
    color: #8a8a8a;
}

.detail-value,
.metric-value,
.column-title,
.card-title,
.workspace-name {
    color: #e4e4e4;
}

.sidebar {
    background-color: @lc-surface-raised;
    border-right: 1px solid @lc-border;
    padding-top: 10px;
}

.sidebar-chrome {
    padding: 0 10px 8px 10px;
}

.sidebar-chrome-button,
.sidebar-icon-button,
.sidebar-reopen-button,
.sidebar-arrow-button {
    background: transparent;
    border: none;
    box-shadow: none;
    text-shadow: none;
    padding: 0;
    min-width: 28px;
    min-height: 28px;
    border-radius: 7px;
    color: #8a8a8a;
}

.sidebar-chrome-button:hover,
.sidebar-icon-button:hover,
.sidebar-reopen-button:hover,
.sidebar-arrow-button:hover {
    background-color: #2c2c2c;
    color: #e4e4e4;
}

.sidebar-nav-group {
    padding: 8px 10px 6px 10px;
}

.sidebar-nav-button {
    background: transparent;
    border: none;
    box-shadow: none;
    text-shadow: none;
    padding: 0;
    min-height: 32px;
    margin: 0 0 2px 0;
    border-radius: 8px;
}

.sidebar-nav-button:hover {
    background-color: #2c2c2c;
}

.sidebar-nav-button image {
    min-width: 16px;
}

.sidebar-nav-icon {
    min-width: 16px;
    color: #9a9a9a;
}

.sidebar-nav-label {
    color: #d2d2d2;
    font-size: 13px;
    font-weight: 500;
}

.projects-header {
    padding: 12px 6px 6px 10px;
}

.nav-row,
.nav-row-active,
.nav-button,
.nav-button-active {
    margin: 0;
    padding: 9px 12px;
    border-radius: 7px;
    background: transparent;
    border: 1px solid transparent;
    box-shadow: none;
    text-shadow: none;
    font-size: 14px;
    font-weight: 500;
}

.nav-row,
.nav-button {
    color: #b4b4b4;
}

.nav-row-active,
.nav-button-active {
    color: #e4e4e4;
    background-color: #2e2e2e;
    border-color: transparent;
}

.nav-button:hover,
.nav-row:hover {
    color: #e4e4e4;
    background-color: #2c2c2c;
    border-color: transparent;
}

.sidebar-search,
.composer-bar entry,
entry {
    background-color: #151515;
    color: #e4e4e4;
    border: 1px solid #343434;
    border-radius: 8px;
    font-size: 13px;
    padding: 7px 10px;
    caret-color: #f5f5f5;
}

.sidebar-search:focus,
.composer-bar entry:focus,
entry:focus {
    border-color: #8a8a8a;
    box-shadow: 0 0 0 1px rgba(180, 180, 180, 0.26);
    outline: 1px solid transparent;
    outline-offset: 2px;
}

entry placeholder,
textview placeholder {
    color: #737373;
}

entry:disabled,
combobox:disabled,
textview:disabled {
    background-color: #121212;
    color: #6f6f6f;
    border-color: #242424;
}

.workspace-list {
    background-color: transparent;
}

.workspace-list row {
    border-radius: 7px;
    margin: 2px 6px 2px 10px;
    padding: 0;
}

.workspace-list row:hover {
    background-color: @lc-hover;
}

.workspace-list row:hover .workspace-row-shell,
.workspace-list row:selected .workspace-row-shell {
    background-color: @lc-hover-soft;
    border-color: @lc-border-strong;
    padding-right: 14px;
}

.workspace-row-shell,
.project-row,
.history-row {
    padding: 8px 10px;
}

.workspace-row-diff-added,
.workspace-row-diff-removed {
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 11px;
    font-weight: 700;
}

.workspace-row-diff-added {
    color: @lc-success;
}

.workspace-row-diff-removed {
    color: @lc-danger;
}

.workspace-row-branch-icon {
    min-width: 14px;
    color: @lc-text-muted;
}

.workspace-name {
    font-size: 14px;
    font-weight: 600;
}

.workspace-meta,
.card-meta,
.detail-label,
.repo-section-header {
    font-size: 11px;
}

.repo-section-header {
    letter-spacing: 0.1em;
}

.repo-empty-label {
    padding: 4px 12px 10px 12px;
}

.workspace-status-chip,
.workspace-status-chip-active {
    padding: 1px 0;
    border-radius: 7px;
    font-size: 11px;
    font-weight: 600;
    min-width: 0;
}

.workspace-status-chip {
    color: #b8b8b8;
    background-color: transparent;
    border: none;
}

.workspace-status-chip-active {
    color: #e4e4e4;
    background-color: transparent;
    border: none;
}

.project-icon {
    color: #8a8a8a;
}

.project-icon-hot,
.card-diff-hot,
.run-dot-active,
.stat-running {
    color: #aaaaaa;
}

.app-header,
headerbar {
    background-color: #1e1e1e;
    border-bottom: 1px solid #2a2a2a;
}

.chrome-button {
    border-radius: 7px;
    background: transparent;
    border: 1px solid transparent;
}

button {
    background-image: none;
    background-color: transparent;
    color: #e4e4e4;
    border: 1px solid transparent;
    border-radius: 7px;
    box-shadow: none;
    text-shadow: none;
    padding: 7px 12px;
    min-height: 34px;
    font-family: "Mona Sans", "Adwaita Sans", "SF Pro Text", "Segoe UI", "Cantarell", "Noto Sans", sans-serif;
    font-size: 13px;
    font-weight: 500;
    letter-spacing: 0;
}

button.text-button,
button.icon-button {
    background-image: none;
    background-color: transparent;
    color: #e4e4e4;
    border: 1px solid transparent;
    border-radius: 7px;
    box-shadow: none;
    text-shadow: none;
    font-family: "Mona Sans", "Adwaita Sans", "SF Pro Text", "Segoe UI", "Cantarell", "Noto Sans", sans-serif;
    font-size: 13px;
    font-weight: 500;
    letter-spacing: 0;
}

button.text-button {
    padding: 7px 12px;
    min-height: 34px;
}

button.icon-button {
    padding: 0;
    min-width: 30px;
    min-height: 30px;
}

button.icon-button image {
    min-width: 16px;
}

button:hover {
    background-color: rgba(255, 255, 255, 0.06);
    border-color: transparent;
}

button:active {
    background-color: rgba(255, 255, 255, 0.1);
}

button.suggested-action {
    background-color: #282828;
    color: #e4e4e4;
    border-color: transparent;
}

button.suggested-action:hover {
    background-color: #333333;
    border-color: transparent;
}

button.secondary-action {
    background-color: #202020;
    color: #b4b4b4;
    border-color: #2a2a2a;
}

button.secondary-action:hover {
    background-color: #2c2c2c;
    color: #e4e4e4;
    border-color: #2a2a2a;
}

button.flat-action {
    background-image: none;
    background-color: transparent;
    color: #b4b4b4;
    border-color: transparent;
}

button.flat-action:hover {
    background-color: #2c2c2c;
    color: #e4e4e4;
    border-color: transparent;
}

button.destructive-action {
    background-color: #3a1f24;
    color: #e4e4e4;
    border-color: transparent;
}

button.destructive-action:hover {
    background-color: #4a252c;
    border-color: transparent;
}

checkbutton {
    color: #b4b4b4;
}

.chrome-button:hover {
    background-color: #2c2c2c;
    border-color: transparent;
}

.project-tabs {
    padding-bottom: 8px;
}

.project-tab,
.project-tab-active {
    font-size: 13px;
    font-weight: 600;
    padding-bottom: 10px;
}

.project-tab-active,
.card-activity,
.workspace-title {
    color: #aaaaaa;
}

.project-tab-active {
    border-bottom: 2px solid #aaaaaa;
}

.kanban-column {
    min-width: 250px;
    padding: 12px;
    border-radius: 12px;
    background-color: #181818;
    border: 1px solid #2b2b2b;
}

.kanban-board {
    background-color: #101010;
}

.dashboard .page-board {
    padding: 16px 20px;
}

.dashboard .kanban-column {
    padding: 8px;
    border: 1px solid #2b2b2b;
}

.workspace-card-action {
    padding: 0;
    background: transparent;
    border: none;
    box-shadow: none;
}

.workspace-card-action .workspace-card {
    border: 1px solid #303030;
    box-shadow: none;
}

.workspace-card-action:focus-visible {
    outline: 2px solid #aaaaaa;
    outline-offset: 2px;
}

.kanban-column-header {
    border-bottom: 1px solid #2b2b2b;
    padding-bottom: 8px;
    margin-bottom: 2px;
}

.shell-card,
.workspace-card,
.command-panel,
.metric-card,
.detail-row,
.settings-panel {
    background-color: #202020;
    border: 1px solid #2a2a2a;
    border-radius: 14px;
}

.workspace-card,
.command-panel,
.metric-card,
.detail-row {
    padding: 12px;
}

.workspace-card {
    min-height: 104px;
}

.kanban-column .workspace-card {
    background-color: #151515;
    border-color: #303030;
    border-radius: 10px;
    padding: 10px;
    box-shadow: none;
}

.kanban-column .workspace-card:hover {
    background-color: #1d1d1d;
    border-color: #454545;
}

.dashboard-card-top,
.dashboard-card-footer {
    min-height: 22px;
}

.card-branch,
.card-diff,
.column-empty,
.empty-label,
.status-detail,
.info-text {
    color: #8a8a8a;
}

.metric-value {
    font-size: 16px;
    font-weight: 700;
}
.card-meta.ws-pr-status-ready {
    color: #cfcfcf;
}
.card-meta.ws-pr-status-pending {
    color: #a8a8a8;
}
.card-meta.ws-pr-status-failed {
    color: #ff8a8a;
}
.card-meta.ws-pr-status-merged {
    color: #c6a3ff;
}

.command-center-strip,
.workspace-summary-strip {
    padding: 0;
    margin-bottom: 8px;
}

.panel-switcher {
    background-color: transparent;
}

.panel-switcher button {
    background-color: transparent;
    color: #8a8a8a;
    border: 1px solid transparent;
    border-radius: 7px;
    padding: 6px 12px;
    font-size: 12px;
    font-weight: 600;
}

.panel-switcher button:hover {
    background-color: #2c2c2c;
    color: #e4e4e4;
    border-color: transparent;
}

.panel-switcher button:checked {
    background-color: #2e2e2e;
    color: #e4e4e4;
    border-color: transparent;
}

.terminal-panel,
.session-tool-surface,
.session-transcript,
.terminal-transcript-dark,
.checks-view,
.diff-view,
.status-container {
    background-color: #151515;
    color: #e4e4e4;
    border: 1px solid #2a2a2a;
    border-radius: 14px;
}

.terminal-panel,
.session-tool-surface,
.session-surface {
    padding: 12px;
}

.terminal-panel .history-view,
.session-transcript,
.terminal-transcript-dark,
.checks-view,
.diff-view {
    background-color: #151515;
    color: #e4e4e4;
    font-size: 12px;
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
}

.session-surface .card-meta,
.session-tool-surface .card-meta,
.terminal-panel .card-meta {
    color: #8a8a8a;
}

.terminal-tab-strip button,
.pill-button {
    border-radius: 7px;
}

.terminal-tab-strip button {
    font-size: 12px;
    padding: 5px 10px;
}

.pill-button {
    background-color: #2c2c2c;
    color: #b4b4b4;
    border: 1px solid transparent;
    padding: 4px 9px;
    font-size: 12px;
    font-weight: 600;
}

.pill-button:hover {
    background-color: #2e2e2e;
}

.composer-bar {
    background-color: #1e1e1e;
    border-top: 1px solid #2a2a2a;
}

separator {
    background-color: #2a2a2a;
    min-width: 1px;
    min-height: 1px;
}

.mini-action-button {
    min-width: 28px;
    min-height: 28px;
    border-radius: 7px;
    background-color: transparent;
    color: #8a8a8a;
    border: none;
    font-weight: 700;
}

.mini-action-button:hover {
    background-color: rgba(255, 255, 255, 0.06);
    color: #e4e4e4;
}

.repo-section-row {
    padding: 0;
}

.repo-section-icon,
.workspace-row-icon {
    color: @lc-text-muted;
}

.workspace-row-icon-active {
    color: @lc-text;
}

.repo-section-header {
    color: @lc-text;
    font-size: 13px;
    font-weight: 500;
    letter-spacing: 0;
    text-transform: none;
}

.repo-section-count {
    color: @lc-text-muted;
    font-size: 13px;
}

.repo-header-add {
    min-width: 24px;
    min-height: 24px;
    padding: 0;
    color: @lc-text-muted;
    border: none;
}

.repo-header-add:hover {
    color: @lc-text;
}

.settings-cta {
    background-color: #202020;
    border: 1px solid #2a2a2a;
    border-radius: 12px;
    padding: 12px 14px;
}

.action-row,
.project-actions-row,
.workspace-title-row {
    margin-top: 2px;
    margin-bottom: 2px;
}

.action-stack {
    background-color: #202020;
    border: 1px solid #2a2a2a;
    border-radius: 12px;
    padding: 10px;
}

.toolbar-label {
    color: #8a8a8a;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
}

.action-input-row {
    background-color: transparent;
}

.surface-note {
    color: #8a8a8a;
    font-size: 12px;
}

.modal-body {
    background-color: #1e1e1e;
    border: 1px solid #2a2a2a;
    border-radius: 14px;
    padding: 18px;
}

.workspace-modal {
    padding: 18px;
}

.setup-modal {
    background-color: #1a1a1a;
    border: 1px solid #2f2f2f;
    border-radius: 14px;
    padding: 18px;
}

.setup-title {
    color: #f2f2f2;
    font-size: 20px;
    font-weight: 700;
}

.setup-copy,
.setup-feedback,
.setup-status-detail {
    color: #9a9a9a;
    font-size: 12px;
}

.setup-status-list,
.setup-guidance {
    background-color: #151515;
    border: 1px solid #2a2a2a;
    border-radius: 12px;
    padding: 10px;
}

.setup-status-row {
    border-radius: 10px;
    padding: 10px;
}

.setup-status-ready {
    background-color: #181818;
    border: 1px solid #2b2b2b;
}

.setup-status-missing-required {
    background-color: #24191a;
    border: 1px solid #4a2b2f;
}

.setup-status-missing {
    background-color: #181818;
    border: 1px solid #2b2b2b;
}

.setup-status-pill {
    min-width: 58px;
    color: #e4e4e4;
    background-color: #2c2c2c;
    border-radius: 999px;
    padding: 3px 8px;
    font-size: 11px;
    font-weight: 700;
}

.setup-status-name {
    color: #f2f2f2;
    font-size: 13px;
    font-weight: 700;
}

.setup-link {
    color: #f2f2f2;
    background-color: #202020;
    border: 1px solid #303030;
    border-radius: 9px;
    padding: 9px 10px;
}

.setup-link:hover {
    background-color: #2a2a2a;
    border-color: #3a3a3a;
}

.project-create-menu {
    background-color: #1e1e1e;
    border: 1px solid #343434;
    border-radius: 10px;
}

.project-create-menu-row {
    min-width: 260px;
    min-height: 34px;
    padding: 6px 8px;
    border-radius: 7px;
    background-color: transparent;
    border: 1px solid transparent;
}

.project-create-menu-row:hover {
    background-color: #2c2c2c;
}

.project-create-menu-icon {
    color: #a8a8a8;
    min-width: 16px;
}

.project-create-menu-label {
    color: #e4e4e4;
    font-size: 13px;
    font-weight: 500;
}

.project-folder-picker,
.project-repo-list,
.project-template-grid {
    background-color: #151515;
    border: 1px solid #2a2a2a;
    border-radius: 12px;
    padding: 10px;
}

.project-path-preview,
.project-template-copy {
    color: #8a8a8a;
    font-size: 12px;
}

.project-repo-row {
    border-radius: 9px;
    margin: 2px;
}

.project-repo-row:hover,
.project-repo-row:selected {
    background-color: #262626;
}

.project-template-card {
    background-color: #1b1b1b;
    border: 1px solid #303030;
    border-radius: 10px;
    padding: 0;
}

.project-template-card:hover {
    background-color: #242424;
    border-color: #454545;
}

.project-template-card-selected {
    background-color: #2a2a2a;
    border-color: #6a6a6a;
}

.project-template-title {
    color: #f2f2f2;
    font-size: 13px;
    font-weight: 700;
}

.workspace-modal-split {
    margin-top: 10px;
    margin-bottom: 2px;
}

.workspace-modal-field {
    min-height: 40px;
    min-width: 0;
}

.workspace-modal-section {
    background-color: #181818;
    border: 1px solid #2b2b2b;
    border-radius: 10px;
    padding: 12px;
    margin-top: 4px;
}

.workspace-modal-section-title {
    color: #f2f2f2;
    font-size: 12px;
    font-weight: 700;
}

.workspace-modal-hint {
    margin-top: 6px;
    margin-bottom: 4px;
}

.workspace-modal-preview {
    background-color: #151515;
    border: 1px solid #2a2a2a;
    border-radius: 12px;
    padding: 12px;
    margin-top: 8px;
}

.workspace-modal-preview-copy {
    line-height: 1.45;
}

.workspace-modal-feedback {
    margin-top: 6px;
}

.settings-shell {
    background-color: #1e1e1e;
    border: 1px solid #2a2a2a;
    border-radius: 14px;
    padding: 14px;
}

.settings-toolbar {
    background-color: #1e1e1e;
    border-bottom: 1px solid #2a2a2a;
    padding-bottom: 12px;
}

.settings-toolbar-row {
    margin: 0;
}

.settings-status {
    color: #8a8a8a;
    font-size: 12px;
}

.settings-inspector {
    margin-top: 2px;
}

.settings-rail {
    background-color: #202020;
    border: 1px solid #2a2a2a;
    border-radius: 14px;
    padding: 8px;
}

.settings-rail-button,
.settings-rail-button-active {
    background-color: transparent;
    border: 1px solid transparent;
    border-radius: 7px;
    box-shadow: none;
    text-shadow: none;
    padding: 9px 10px;
}

.settings-rail-button {
    color: #b4b4b4;
}

.settings-rail-button:hover {
    background-color: #2c2c2c;
    border-color: transparent;
}

.settings-rail-button-active {
    color: #e4e4e4;
    background-color: #2e2e2e;
    border-color: transparent;
}

.settings-rail-title {
    color: #e4e4e4;
    font-size: 13px;
    font-weight: 600;
}

.settings-rail-copy {
    color: #8a8a8a;
    font-size: 11px;
}

.settings-content-shell {
    background-color: transparent;
    border: none;
    border-radius: 0;
    padding: 0;
}

.settings-content-panel {
    background-color: transparent;
    border: none;
    box-shadow: none;
    padding: 0;
}

.settings-group {
    background-color: #202020;
    border: 1px solid #2a2a2a;
    border-radius: 14px;
    padding: 14px;
}

.settings-group-title {
    color: #e4e4e4;
    font-size: 14px;
    font-weight: 600;
}

.settings-group-copy {
    color: #8a8a8a;
    font-size: 12px;
}

.settings-group-body {
    margin-top: 2px;
}

.settings-field-row {
    margin-top: 2px;
}

.settings-field,
.settings-editor-field,
.settings-toggle-row {
    background-color: transparent;
    border: none;
    border-bottom: 1px solid #2a2a2a;
    border-radius: 0;
    padding: 10px 0 12px;
}

.settings-field-title {
    color: #e4e4e4;
    font-size: 13px;
    font-weight: 500;
}

.settings-field-copy {
    color: #8a8a8a;
    font-size: 12px;
}

.settings-inherited-label {
    color: #aaaaaa;
    font-size: 11px;
    font-style: italic;
}

.settings-machine-entry,
.settings-editor,
.settings-editor text,
.settings-editor view,
.settings-editor widget {
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
}

.settings-editor-shell {
    background-color: #151515;
    border: 1px solid #2a2a2a;
    border-radius: 10px;
}

.settings-editor {
    background-color: #151515;
    color: #e4e4e4;
    padding: 8px;
}

.settings-shell entry,
.settings-shell combobox,
.settings-shell textview,
.settings-shell scrolledwindow {
    border-radius: 7px;
}

.settings-shell checkbutton {
    color: #e4e4e4;
}

.lc-accent-blue .section-title,
.lc-accent-blue .project-tab-active,
.lc-accent-blue .card-activity,
.lc-accent-blue .workspace-title {
    color: #f5f5f5;
    border-color: #5a5a5a;
}

.lc-accent-green .section-title,
.lc-accent-green .project-tab-active,
.lc-accent-green .card-activity,
.lc-accent-green .workspace-title {
    color: #f5f5f5;
    border-color: #5a5a5a;
}

.lc-accent-amber .section-title,
.lc-accent-amber .project-tab-active,
.lc-accent-amber .card-activity,
.lc-accent-amber .workspace-title {
    color: #fff7ed;
    border-color: #b35c00;
}

.lc-accent-rose .section-title,
.lc-accent-rose .project-tab-active,
.lc-accent-rose .card-activity,
.lc-accent-rose .workspace-title {
    color: #fff1f2;
    border-color: #be123c;
}

.lc-density-compact .project-row,
.lc-density-compact .history-row,
.lc-density-compact .detail-row,
.lc-density-compact .command-panel,
.lc-density-compact .metric-card,
.lc-density-compact .workspace-card {
    padding: 8px;
}

.lc-density-compact .detail-body,
.lc-density-compact .page-body,
.lc-density-compact .kanban-board,
.lc-density-compact .page-board {
    padding: 18px 22px;
}

.lc-density-comfortable .project-row,
.lc-density-comfortable .history-row,
.lc-density-comfortable .detail-row,
.lc-density-comfortable .command-panel,
.lc-density-comfortable .metric-card,
.lc-density-comfortable .workspace-card {
    padding: 14px;
}

.lc-density-comfortable .detail-body,
.lc-density-comfortable .page-body,
.lc-density-comfortable .kanban-board,
.lc-density-comfortable .page-board {
    padding: 32px;
}

.lc-theme-dark .dashboard,
.lc-theme-dark .page-shell,
.lc-theme-dark .history-view {
    background-color: #191919;
    color: #e4e4e4;
}

.lc-theme-dark .page-header {
    background-color: #1e1e1e;
    border-color: #2a2a2a;
}

.lc-theme-dark .workspace-card,
.lc-theme-dark .command-panel,
.lc-theme-dark .metric-card,
.lc-theme-dark .detail-row,
.lc-theme-dark .settings-panel {
    background-color: #202020;
    border-color: #2a2a2a;
}

.lc-theme-dark .dashboard-title,
.lc-theme-dark .workspace-name,
.lc-theme-dark .card-title,
.lc-theme-dark .metric-value,
.lc-theme-dark .detail-value,
.lc-theme-dark .column-title {
    color: #e4e4e4;
}

.lc-theme-dark .card-meta,
.lc-theme-dark .workspace-meta,
.lc-theme-dark .detail-label,
.lc-theme-dark .project-tab,
.lc-theme-dark .column-count,
.lc-theme-dark .empty-label,
.lc-theme-dark .card-branch,
.lc-theme-dark .card-diff {
    color: #8a8a8a;
}

.lc-theme-dark .composer-bar entry,
.lc-theme-dark entry {
    background-color: #191919;
    color: #e4e4e4;
    border-color: #2a2a2a;
}

.lc-theme-dark .chrome-button:hover,
.lc-theme-dark .panel-switcher button:hover,
.lc-theme-dark .panel-switcher button:checked {
    background-color: #2e2e2e;
    border-color: transparent;
    color: #e4e4e4;
}

.lc-theme-dark .settings-shell {
    background-color: #1e1e1e;
    border-color: #2a2a2a;
}

.lc-theme-dark .settings-toolbar,
.lc-theme-dark .settings-content-shell,
.lc-theme-dark .settings-group,
.lc-theme-dark .settings-rail {
    border-color: #2a2a2a;
}

.lc-theme-light .dashboard,
.lc-theme-light .page-shell,
.lc-theme-light .history-view {
    background-color: #f8fafc;
    color: #1f2937;
}

.lc-theme-light .page-header {
    background-color: #ffffff;
    border-color: #d9e0e8;
}

.lc-theme-light .workspace-card,
.lc-theme-light .command-panel,
.lc-theme-light .metric-card,
.lc-theme-light .detail-row,
.lc-theme-light .settings-panel {
    background-color: #ffffff;
    border-color: #d9e0e8;
}

.lc-theme-light .dashboard-title,
.lc-theme-light .workspace-name,
.lc-theme-light .card-title,
.lc-theme-light .metric-value,
.lc-theme-light .detail-value,
.lc-theme-light .column-title {
    color: #111827;
}

.lc-theme-light .card-meta,
.lc-theme-light .workspace-meta,
.lc-theme-light .detail-label,
.lc-theme-light .project-tab,
.lc-theme-light .column-count,
.lc-theme-light .empty-label,
.lc-theme-light .card-branch,
.lc-theme-light .card-diff {
    color: #667085;
}

.lc-theme-light .composer-bar entry,
.lc-theme-light entry {
    background-color: #ffffff;
    color: #1f2937;
    border-color: #d9e0e8;
}

.lc-theme-light .chrome-button:hover,
.lc-theme-light .panel-switcher button:hover,
.lc-theme-light .panel-switcher button:checked {
    background-color: #edf2f7;
    border-color: transparent;
    color: #111827;
}

/* ── Sidebar status dots ── */
.status-dot {
    min-width: 8px;
    min-height: 8px;
    border-radius: 50%;
    background-color: #8a8a8a;
    opacity: 0.5;
    margin-top: 5px;
}
.status-dot-active {
    background-color: #aaaaaa;
    opacity: 1.0;
}
.status-dot-running {
    background-color: #aaaaaa;
    opacity: 1.0;
}
.status-dot-idle {
    background-color: #8a8a8a;
    opacity: 0.45;
}

/* ── Sidebar row timestamp ── */
.workspace-row-timestamp {
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 11px;
    color: #8a8a8a;
}

/* ── Sidebar bottom bar ── */
.sidebar-bottom-bar {
    background-color: #1e1e1e;
    border-top: 1px solid #2a2a2a;
    padding: 6px 10px;
    min-height: 44px;
}

/* ── Section header ── */
.repo-section-row {
    padding: 8px 6px 4px 10px;
}
.repo-section-header {
    font-size: 12px;
    font-weight: 600;
    color: @lc-text;
    letter-spacing: 0;
}
.repo-section-icon {
    min-width: 14px;
    color: @lc-text-muted;
}

/* ── Updated workspace row shell ── */
.workspace-row-shell {
    padding: 7px 14px 7px 10px;
}

.sidebar-icon-button {
    min-width: 24px;
    min-height: 24px;
}

.sidebar-window-button {
    min-width: 12px;
    min-height: 12px;
    padding: 0;
    border-radius: 999px;
    color: #7a7a7a;
}

.sidebar-window-button:hover {
    color: #7a7a7a;
}

.sidebar-arrow-button {
    min-width: 24px;
    min-height: 24px;
    color: #9a9a9a;
}

.sidebar-arrow-button:hover {
    background-color: #2c2c2c;
    color: #e4e4e4;
}

.sidebar-chrome .sidebar-window-button:nth-child(1):hover {
    background-color: #ff5f57;
}

.sidebar-chrome .sidebar-window-button:nth-child(2):hover {
    background-color: #febc2e;
}

.sidebar-chrome .sidebar-window-button:nth-child(3):hover {
    background-color: #28c840;
}

/* ── Workspace empty state ── */
.workspace-empty-label {
    font-size: 14px;
    color: #8a8a8a;
}

/* ── Workspace title bar ── */
.ws-title-bar {
    background-color: #1e1e1e;
    border-bottom: 1px solid #000;
    padding: 0 14px;
    min-height: 46px;
}
.ws-breadcrumb {
    font-size: 14px;
    font-weight: 500;
    color: #e0e0e0;
}
.ws-pr-badge {
    border-radius: 7px;
}
.ws-pr-num {
    font-size: 13px;
    font-weight: 600;
    color: #e0e0e0;
    background-color: #2e2e2e;
    padding: 5px 11px;
}
.ws-pr-sep {
    min-width: 1px;
    background-color: #252525;
}
.ws-pr-state {
    font-size: 13px;
    font-weight: 600;
    color: #c8c8c8;
    background-color: #252525;
    padding: 5px 13px;
}

/* ── Center panel / tab bar ── */
.ws-center {
    background-color: #191919;
}
.ws-tab-bar {
    min-height: 46px;
    padding: 0 18px;
    background-color: #191919;
}
.ws-chat-tabs-scroll {
    background-color: transparent;
    border: none;
    box-shadow: none;
    min-height: 46px;
}
.ws-chat-tabs-scroll scrollbar {
    min-height: 0;
    min-width: 0;
}
.ws-chat-tabs {
    background-color: transparent;
}
.ws-tab-sep {
    min-height: 1px;
    background-color: #282828;
}
button.ws-tab-shell {
    background-image: none;
    border: none;
    border-radius: 0;
    box-shadow: none;
    text-shadow: none;
}
.ws-tab-shell {
    min-width: 132px;
    min-height: 46px;
    padding: 0 6px;
    border-bottom: 2px solid transparent;
    background-color: transparent;
}
.ws-tab-label {
    min-width: 84px;
    font-size: 14px;
    font-weight: 400;
    color: #8a8a8a;
}
.ws-tab-close-button {
    background: transparent;
    border: none;
    box-shadow: none;
    text-shadow: none;
    border-radius: 5px;
    padding: 0;
    min-width: 24px;
    min-height: 24px;
}
.ws-tab-close-button:hover {
    background-color: rgba(255, 255, 255, 0.06);
}
.ws-tab-close-icon {
    color: #747474;
    min-width: 14px;
    min-height: 14px;
}
.ws-tab-shell:hover .ws-tab-label,
.ws-tab-shell:hover .ws-tab-close-icon {
    color: #c6c6c6;
}
.ws-tab-shell.ws-tab-active {
    background-color: #242424;
    border-bottom-color: #f0c36a;
}
.ws-tab-shell.ws-tab-active .ws-tab-label,
.ws-tab-shell.ws-tab-active .ws-tab-close-icon {
    color: #e8e8e8;
    font-weight: 600;
}
.ws-mode-switcher button {
    font-family: "Mona Sans", "Adwaita Sans", "SF Pro Text", "Segoe UI", "Cantarell", "Noto Sans", sans-serif;
    font-size: 12px;
    padding: 3px 10px;
    border-radius: 5px;
}

/* ── Right panel ── */
.ws-right-panel {
    background-color: #151515;
    border-left: 1px solid #000;
}
.ws-right-tabs {
    min-height: 46px;
    padding: 0 10px;
    background-color: #151515;
    border-bottom: 1px solid #232323;
}
.ws-right-tab-btn {
    background: transparent;
    border: none;
    border-radius: 7px;
    padding: 4px 11px;
    font-family: "Mona Sans", "Adwaita Sans", "SF Pro Text", "Segoe UI", "Cantarell", "Noto Sans", sans-serif;
    font-size: 13px;
    color: #8e8e8e;
}
.ws-right-tab-btn:hover {
    background-color: #1e1e1e;
    color: #cacaca;
}
.ws-right-tab-active {
    background-color: #232323;
    color: #e8e8e8;
}
.chat-header-row,
.ws-pr-nav,
.ws-changes-header,
.ws-run-header {
    min-height: 42px;
    padding: 10px 12px;
    background-color: #181818;
    border-bottom: 1px solid #232323;
}
.ws-changes-title {
    font-size: 13px;
    font-weight: 700;
    color: #e6e6e6;
}
.ws-pr-number {
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 13px;
    font-weight: 700;
    color: #d9d9d9;
}
.ws-pr-status {
    font-size: 12px;
    font-weight: 700;
    border-radius: 999px;
    padding: 4px 10px;
    background-color: #262626;
    color: #b6b6b6;
}
.ws-pr-status-muted {
    background-color: #262626;
    color: #a8a8a8;
}
.ws-pr-status-ready {
    background-color: #262626;
    color: #cfcfcf;
}
.ws-pr-status-pending {
    background-color: #262626;
    color: #a8a8a8;
}
.ws-pr-status-failed {
    background-color: #3a1a1a;
    color: #ff8a8a;
}
.ws-pr-status-merged {
    background-color: #311d46;
    color: #c6a3ff;
}
.ws-pr-status-missing {
    background-color: #1b1b1b;
    color: #a8a8a8;
}
.ws-pr-compact-panel {
    min-height: 42px;
    padding: 10px 12px;
    border-bottom: 1px solid #232323;
    background-color: #181818;
}
.ws-pr-compact-panel.ws-pr-status-muted,
.ws-pr-compact-panel.ws-pr-status-missing {
    background-color: #181818;
    border-bottom-color: #232323;
}
.ws-pr-compact-panel.ws-pr-status-pending {
    background-color: #181818;
    border-bottom-color: #2d2d2d;
}
.ws-pr-compact-panel.ws-pr-status-ready {
    background-color: #181818;
    border-bottom-color: #2d2d2d;
}
.ws-pr-compact-panel.ws-pr-status-failed {
    background-color: #2a1718;
    border-bottom-color: #5c2529;
}
.ws-pr-compact-panel.ws-pr-status-merged {
    background-color: #251935;
    border-bottom-color: #533379;
}
.ws-pr-compact-title {
    font-size: 13px;
    font-weight: 700;
    color: #e6e6e6;
}
.ws-pr-action-button {
    min-height: 32px;
    padding: 0 14px;
    font-family: "Mona Sans", "Adwaita Sans", "SF Pro Text", "Segoe UI", "Cantarell", "Noto Sans", sans-serif;
    font-size: 13px;
}
.ws-pr-action-button.ws-pr-status-muted,
.ws-pr-action-button.ws-pr-status-missing,
.ws-pr-action-button.ws-pr-status-pending {
    background-color: #262626;
    color: #a8a8a8;
    border-color: #3a3a3a;
}
.ws-pr-action-button.ws-pr-status-ready {
    background-color: #303030;
    color: #f5f5f5;
    border-color: #4a4a4a;
}
.ws-pr-action-button.ws-pr-status-failed {
    background-color: #8d2e34;
    color: #fff5f5;
    border-color: #b3444b;
}
.ws-pr-action-button.ws-pr-status-merged {
    background-color: #6d3fa0;
    color: #fbf7ff;
    border-color: #8655bd;
}
.ws-changes-menu-btn,
.ws-run-collapse-btn {
    background: transparent;
    border: none;
    box-shadow: none;
    color: #8e8e8e;
    font-family: "Mona Sans", "Adwaita Sans", "SF Pro Text", "Segoe UI", "Cantarell", "Noto Sans", sans-serif;
    font-size: 13px;
    min-width: 30px;
    min-height: 30px;
    border-radius: 6px;
}
.ws-changes-menu-btn:hover,
.ws-run-collapse-btn:hover {
    background-color: #242424;
    color: #e4e4e4;
}
.ws-file-summary-panel {
    padding: 10px 12px 14px;
    background-color: #151515;
}
.ws-file-summary-row {
    min-height: 32px;
    padding: 5px 8px;
    border-radius: 6px;
    border: none;
    box-shadow: none;
    background: transparent;
}
.ws-file-summary-row:hover {
    background-color: #1f1f1f;
}
.ws-file-summary-state {
    color: #9a9a9a;
    font-size: 12px;
}
.ws-file-summary-counts {
    color: #bfc7d5;
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 12px;
    font-weight: 700;
}

/* ── File list ── */
.ws-file-list {
    background: transparent;
}
.ws-file-list row {
    min-height: 0;
}
.ws-dir-row {
    padding: 1px 3px;
}
.ws-folder-icon {
    color: #c39b50;
    min-width: 13px;
    min-height: 13px;
}
.ws-folder-toggle {
    min-width: 12px;
    min-height: 13px;
    padding: 0;
    border: none;
    background: transparent;
    color: #777777;
    font-size: 12px;
}
.ws-folder-name {
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 12px;
    font-weight: 500;
    color: #b0b0b0;
}
.ws-file-row {
    padding: 1px 3px;
}
.ws-file-icon {
    color: #8091a7;
    min-width: 13px;
    min-height: 13px;
}
.ws-file-name {
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 12px;
    color: #c0c0c0;
}
.ws-file-dir {
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 11px;
    color: #5d5d5d;
}
.ws-file-list row:selected {
    background-color: #293243;
}
.ws-file-list row:hover {
    background-color: #20242b;
}
row:selected .ws-file-name,
row:selected .ws-folder-name {
    color: #e8e8e8;
}
row:hover .ws-file-name,
row:hover .ws-folder-name {
    color: #cacaca;
}

/* ── Run console section ── */
.ws-run-section {
    background-color: #111111;
}
.ws-run-body {
    padding: 8px 0 0;
}
.ws-run-panel {
    padding: 4px 12px 10px;
}
.ws-run-tab-bar {
    min-height: 38px;
    padding: 0 6px;
    background-color: #1a1a1a;
    border-top: 1px solid #222222;
}
.ws-run-tab-btn {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    color: #7a7a7a;
    font-size: 12px;
    padding: 4px 12px;
    min-height: 36px;
    box-shadow: none;
    text-shadow: none;
}
.ws-run-tab-btn:hover {
    color: #c0c0c0;
    background: rgba(255,255,255,0.04);
}
.ws-run-tab-active {
    color: #e4e4e4;
    border-bottom-color: #e4e4e4;
}
.ws-run-tab-add-btn {
    background: transparent;
    border: none;
    border-radius: 5px;
    color: #5a5a5a;
    font-size: 16px;
    padding: 0 8px;
    min-height: 30px;
    box-shadow: none;
    text-shadow: none;
}
.ws-run-tab-add-btn:hover {
    color: #aaaaaa;
    background: rgba(255,255,255,0.05);
}
.ws-run-collapse-btn {
    background: transparent;
    border: none;
    border-radius: 5px;
    color: #5a5a5a;
    font-size: 16px;
    min-width: 28px;
    min-height: 28px;
    box-shadow: none;
    text-shadow: none;
}
.ws-run-collapse-btn:hover {
    color: #aaaaaa;
    background: rgba(255,255,255,0.05);
}
.ws-prompt-modal {
    background-color: rgba(18, 18, 18, 0.92);
    border: 1px solid #2f2f2f;
    border-radius: 14px;
    padding: 14px 16px;
    min-width: 260px;
}
.ws-prompt-modal .detail-label {
    color: #e4e4e4;
}
.ws-run-output,
.ws-run-output text {
    background-color: #111111;
    color: #c8c8c8;
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 12px;
    border-radius: 0;
}
.ws-run-output-scroll {
    background-color: #111111;
}

/* ── Minimal sidebar search ── */
.sidebar-search-minimal {
    background: transparent;
    border: none;
    border-bottom: 1px solid #2a2a2a;
    border-radius: 0;
    font-size: 12px;
    color: #8a8a8a;
    padding: 6px 12px;
    margin: 0;
}
.sidebar-search-minimal:focus {
    border-bottom-color: #aaaaaa;
    color: #e4e4e4;
}

/* ── Title bar nav / breadcrumb ── */
.ws-nav-btn {
    background: transparent;
    border: none;
    box-shadow: none;
    text-shadow: none;
    color: #7e7e7e;
    padding: 5px 6px;
    min-width: 26px;
    min-height: 26px;
    border-radius: 6px;
    font-size: 14px;
}
.ws-nav-btn:hover {
    background-color: #2c2c2c;
    color: #e4e4e4;
}
.ws-panel-toggle {
    background: transparent;
    border: none;
    box-shadow: none;
    text-shadow: none;
    color: #7e7e7e;
    padding: 5px;
    min-width: 28px;
    min-height: 28px;
    border-radius: 6px;
    margin-right: 6px;
}
.ws-panel-toggle:hover {
    background-color: #2c2c2c;
    color: #e4e4e4;
}
.ws-breadcrumb-sep {
    color: #4a4a4a;
    font-size: 14px;
}
.ws-breadcrumb-dots {
    color: #4a4a4a;
    letter-spacing: 2px;
    font-size: 13px;
}

/* ── Chat surface ── */
.chat-surface {
    background-color: #151515;
}
.chat-repo-icon,
.chat-editor-icon,
.chat-mode-icon,
.chat-menu-item-icon,
.chat-focus-btn image,
.chat-toolbar-btn image {
    color: #9a9a9a;
}
.chat-repo-label,
.chat-branch-label {
    color: #e4e4e4;
    font-size: 14px;
    font-weight: 600;
}
.chat-branch-separator {
    color: #5f5f5f;
    font-size: 16px;
    font-weight: 700;
}
.chat-editor-menu,
.chat-mode-menu,
.chat-mode-btn,
.chat-focus-btn,
.chat-context-btn,
.chat-toolbar-btn,
.chat-send-btn {
    border: none;
    box-shadow: none;
    text-shadow: none;
    border-radius: 10px;
    min-width: 0;
    min-height: 0;
}
.chat-editor-menu,
.chat-mode-menu,
.chat-mode-btn {
    background-color: transparent;
    color: #8a8a8a;
    padding: 5px 6px;
}
.chat-mode-menu {
    padding: 0 6px;
}
.chat-mode-menu:hover,
.chat-mode-btn:hover,
.chat-editor-menu:hover,
.chat-focus-btn:hover,
.chat-context-btn:hover,
.chat-toolbar-btn:hover {
    background-color: #2a2a2a;
}
.chat-mode-selected {
    background-color: #2a2a2a;
    color: #f4f7ff;
}
.chat-mode-label,
.chat-editor-label {
    color: inherit;
    font-size: 12px;
    font-weight: 600;
}
.chat-editor-icon,
.chat-mode-icon {
    min-width: 14px;
}
.chat-mode-glyph {
    min-width: 14px;
    color: #9a9a9a;
    font-size: 14px;
    font-weight: 700;
    line-height: 1;
}
.chat-mode-arrow {
    color: #7f7f7f;
}
.chat-menu-popover {
    background-color: #1e1e1e;
    border: 1px solid #313131;
    border-radius: 14px;
}
popover.chat-menu-popover contents {
    background-color: #1e1e1e;
    border: none;
    border-radius: 10px;
    box-shadow: 0 10px 24px rgba(0, 0, 0, 0.32);
}
.context-menu-popover {
    background-color: transparent;
    border: none;
    box-shadow: none;
}
popover.context-menu-popover contents {
    background-color: #1e1e1e;
    border: none;
    border-radius: 10px;
    box-shadow: 0 10px 24px rgba(0, 0, 0, 0.32);
}
popover.context-menu-popover arrow {
    background-color: #1e1e1e;
    border: none;
}
.chat-menu-list {
    padding: 4px;
}
.chat-menu-group-label {
    color: #8f8f8f;
    font-size: 11px;
    font-weight: 700;
    margin: 6px 8px 2px;
}
.chat-menu-item {
    background: transparent;
    border: none;
    box-shadow: none;
    text-shadow: none;
    border-radius: 7px;
    padding: 6px 8px;
    color: #d8d8d8;
    min-height: 30px;
}
.chat-menu-item:hover,
.chat-menu-item-selected {
    background-color: #2a2a2a;
}
.chat-menu-item-label {
    color: inherit;
    text-align: left;
}
.chat-menu-shortcut {
    min-width: 22px;
    padding: 2px 7px;
    border-radius: 999px;
    color: #a0a0a0;
    background-color: #2a2a2a;
    font-size: 11px;
    font-weight: 700;
}
.chat-content-overlay {
    background-color: #151515;
}
.chat-messages {
    padding: 22px 24px 180px;
}
.chat-user-row {
    margin-top: 12px;
    margin-bottom: 10px;
}
.chat-user-bubble {
    background-color: #2e2e2e;
    color: #f4f7ff;
    border-radius: 14px;
    padding: 11px 16px;
    line-height: 1.4;
    min-height: 42px;
}
.chat-queued-row {
    margin-top: 8px;
    margin-bottom: 8px;
    padding: 8px 10px;
    border: 1px solid #2d2d2d;
    border-radius: 6px;
    background-color: #181818;
}
.chat-queued-label {
    color: #8f8f8f;
    font-size: 11px;
    font-weight: 600;
}
.chat-queued-body {
    color: #e2e2e2;
    font-size: 13px;
    line-height: 1.4;
    padding: 0;
}
.chat-agent-text {
    color: #c6c6c6;
    line-height: 1.55;
    margin-bottom: 0;
}
.chat-reasoning-text {
    color: #8f8f8f;
    font-size: 13px;
    line-height: 1.45;
    margin: 0 0 8px 0;
}
.chat-inline-event {
    background-color: transparent;
    border: none;
    border-radius: 0;
    margin-bottom: 0;
    padding: 0;
}
.chat-inline-event-chip {
    background-color: transparent;
    border: none;
    border-radius: 0;
    box-shadow: none;
    color: #e7e7e7;
    font-family: "Mona Sans", "Inter", "Segoe UI", system-ui, sans-serif;
    font-size: 13px;
    font-weight: 500;
    min-height: 22px;
    min-width: 0;
    padding: 2px 0;
}
button.chat-inline-event-chip {
    background-color: transparent;
    border: none;
    box-shadow: none;
    margin: 0;
    min-height: 22px;
    min-width: 0;
    padding: 2px 0;
}
button.chat-inline-event-chip:hover,
button.chat-inline-event-chip:checked {
    background-color: transparent;
    box-shadow: none;
}
.chat-inline-event-chip label {
    margin: 0;
    min-height: 0;
    padding: 0;
}
.chat-inline-event-meta {
    color: #8f8f8f;
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 11px;
}
.chat-inline-event-body {
    background-color: #181818;
    border: 1px solid #2a2a2a;
    border-radius: 5px;
    color: #c9c9c9;
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 10px;
    line-height: 1.25;
    padding: 0 2px;
}
.chat-inline-event-loading {
    color: #b6c7e8;
}
.chat-inline-event-failed {
    color: #f0a8a8;
}
.chat-composer {
    padding: 0 16px 16px;
    background-color: transparent;
}
.chat-queue-overlay {
    margin: 0 8px 6px;
    padding: 4px;
    border: 1px solid #2d2d2d;
    border-radius: 8px;
    background-color: rgba(18, 18, 18, 0.96);
}
.chat-queued-composer-row {
    padding: 5px 6px;
    border-radius: 6px;
    background-color: #181818;
}
.chat-queued-composer-body {
    color: #d6d6d6;
    font-size: 13px;
    padding: 0;
}
.chat-queued-actions {
    opacity: 0;
}
.chat-queued-composer-row:hover .chat-queued-actions,
.chat-queued-composer-row:focus-within .chat-queued-actions,
.chat-queued-actions:hover {
    opacity: 1;
}
.chat-queued-action-btn {
    min-width: 22px;
    min-height: 22px;
    padding: 0;
    border-radius: 5px;
    background-color: transparent;
    color: #a7a7a7;
    border: none;
    box-shadow: none;
}
.chat-queued-action-btn:hover {
    background-color: #2a2a2a;
    color: #f0f0f0;
}
.chat-composer-box {
    border: 1px solid #2a2a2a;
    border-radius: 14px;
    background-color: #202020;
}
.chat-placeholder {
    color: #7f7f7f;
    font-size: 13px;
}
.chat-input-scroll {
    background-color: transparent;
    border: none;
    box-shadow: none;
}
.chat-input-view,
.chat-input-view text {
    background-color: transparent;
    color: #e4e4e4;
    border: none;
    box-shadow: none;
    font-size: 13px;
    min-height: 0;
}
.chat-toolbar {
    padding: 6px 8px 8px;
    background-color: transparent;
}
.chat-footer-toggle .chat-mode-label,
.chat-footer-toggle .chat-mode-arrow {
    color: transparent;
    font-size: 0;
}
.chat-footer-toggle.chat-mode-selected .chat-mode-label,
.chat-footer-toggle.chat-mode-selected .chat-mode-arrow {
    color: inherit;
    font-size: 12px;
}
.chat-toolbar-btn {
    background-color: transparent;
    color: #8a8a8a;
    border: none;
    box-shadow: none;
    text-shadow: none;
    border-radius: 6px;
    padding: 0 6px;
    font-size: 13px;
    min-height: 0;
    min-width: 0;
}
.chat-toolbar-btn:hover {
    background-color: rgba(255, 255, 255, 0.06);
    color: #e4e4e4;
}
.chat-focus-btn,
.chat-context-btn {
    background-color: transparent;
    color: #8a8a8a;
    min-width: 0;
    min-height: 0;
    padding: 0 6px;
}
.chat-context-usage {
    border: 1px solid #2b2b2b;
    border-radius: 8px;
    background-color: #181818;
    color: #a6a6a6;
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 11px;
    font-weight: 700;
    min-width: 60px;
    min-height: 28px;
    padding: 0 6px;
}
.chat-context-usage-label {
    color: inherit;
}
.chat-context-usage-empty {
    background-color: transparent;
    border-color: #303030;
    color: #777777;
}
.chat-context-usage-normal {
    background-color: #181818;
    border-color: #2b2b2b;
    color: #a3a3a3;
}
.chat-context-usage-warning {
    background-color: #2c281b;
    border-color: #6b5c2a;
    color: #e2cf8a;
}
.settings-page .page-body {
    padding: 16px 24px;
}

.settings-page .settings-inspector,
.settings-page .settings-content-panel {
    background-color: transparent;
    border: none;
    box-shadow: none;
}

.settings-page .settings-machine-entry:focus,
.settings-page .settings-editor:focus,
.settings-page .settings-editor-shell:focus-within {
    border-color: #8a8a8a;
    box-shadow: 0 0 0 1px rgba(180, 180, 180, 0.34);
    outline-offset: 2px;
}

.chat-context-usage-danger {
    background-color: #2c1f1f;
    border-color: #753838;
    color: #efb0b0;
}
.chat-context-details {
    min-width: 420px;
    padding: 14px;
}
.chat-send-btn {
    min-width: 0;
    min-height: 0;
    border-radius: 8px;
    background-color: #2e2e2e;
    color: #8a8a8a;
    border: none;
    box-shadow: none;
    text-shadow: none;
    font-size: 16px;
    font-weight: 700;
    padding: 0;
}
.chat-send-btn-active {
    background-color: #333333;
    color: #f5f5f5;
}
.chat-send-btn:hover {
    background-color: #383838;
    color: #e4e4e4;
}
.chat-send-btn-active:hover {
    background-color: #3f3f3f;
    color: #ffffff;
}

/* ── Diff view ── */
.ws-diff-view,
.ws-diff-view text {
    background-color: #131313;
    color: #c0c0c0;
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 12px;
    line-height: 1.5;
}

/* ── Center panel file code view ── */
.ws-file-code-view,
.ws-file-code-view text {
    background-color: #161616;
    color: #c8c8c8;
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 13px;
    line-height: 1.6;
}

/* ── Tab bar add button ── */
.ws-tab-add-btn {
    background: transparent;
    border: none;
    border-radius: 5px;
    color: #555555;
    font-size: 16px;
    padding: 0 8px;
    min-height: 30px;
    box-shadow: none;
    text-shadow: none;
}
.ws-tab-add-btn:hover {
    color: #aaaaaa;
    background: rgba(255,255,255,0.05);
}

/* ── Checks summary panel ── */
.ws-check-summary {
    padding: 6px 4px;
}
.ws-check-row {
    padding: 4px 10px;
    min-height: 28px;
}
.ws-check-row:hover {
    background-color: rgba(255,255,255,0.03);
}
.ws-check-icon {
    font-size: 11px;
    min-width: 22px;
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
}
.ws-check-icon-muted {
    color: #5a5a5a;
}
.ws-check-icon-active {
    color: #aaaaaa;
}
.ws-check-icon-fail {
    color: #bf6060;
}
.ws-check-key {
    font-size: 12px;
    font-weight: 600;
    color: #888888;
    min-width: 140px;
}
.ws-check-val {
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    font-size: 12px;
    color: #c0c0c0;
    padding-left: 8px;
}
.ws-check-sub {
    font-size: 11px;
    color: #666666;
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
    padding-top: 1px;
    padding-bottom: 1px;
}

/* ── 2026 design refresh: graphite developer cockpit ── */
window,
.dashboard,
.page-shell,
.history-view,
.chat-surface,
.session-surface,
.terminal-panel,
.settings-shell {
    background-color: #101010;
    color: #f8fafc;
    font-family: "Mona Sans", "Adwaita Sans", "SF Pro Text", "Segoe UI", "Cantarell", "Noto Sans", sans-serif;
}

.page-header,
.dashboard-header,
.sidebar,
.chat-composer-box,
.settings-toolbar,
.settings-rail,
.workspace-card,
.shell-card,
.kanban-column,
.modal-body,
.workspace-modal,
.terminal-panel,
.session-tool-surface,
.session-transcript,
.terminal-transcript-dark,
.history-row,
.project-row,
.workspace-row-shell {
    background-color: @lc-surface-muted;
    border-color: @lc-border;
    color: @lc-text-strong;
}

.chat-composer-box,
.workspace-card,
.shell-card,
.workspace-modal-preview,
.chat-menu-popover {
    background-color: #1f1f1f;
    border: 1px solid #2b2b2b;
    box-shadow: 0 14px 34px rgba(0, 0, 0, 0.36);
}

.dashboard-title,
.card-title,
.workspace-name,
.detail-value,
.metric-value,
.column-title,
.chat-repo-label,
.chat-branch-label,
.chat-agent-text,
.chat-input-view,
.chat-input-view text,
.settings-group-title,
.settings-field-title {
    color: #f8fafc;
}

.card-meta,
.workspace-meta,
.detail-label,
.project-tab,
.column-count,
.workspace-path-label,
.section-title,
.sidebar-header,
.repo-section-header,
.settings-group-copy,
.settings-field-copy,
.chat-placeholder,
.chat-inline-event-meta,
.chat-mode-label,
.chat-editor-label {
    color: #a3a3a3;
}

.workspace-meta,
.card-branch,
.workspace-path-label,
.detail-label,
.chat-context-usage,
.chat-inline-event-meta,
.chat-inline-event-body,
.terminal-transcript-dark,
.session-transcript,
.history-view,
.settings-editor,
.settings-machine-entry,
.ws-diff-view,
.ws-diff-view text,
.ws-file-code-view,
.ws-file-code-view text {
    font-family: "Commit Mono", "JetBrains Mono", "SF Mono", "Cascadia Mono", "Menlo", monospace;
}

.sidebar-chrome-button,
.sidebar-icon-button,
.sidebar-reopen-button,
.sidebar-arrow-button,
.chat-focus-btn,
.chat-context-btn,
.chat-toolbar-btn,
.chat-mode-btn,
.chat-mode-menu,
.chat-editor-menu,
.icon-button,
.text-button,
.flat-action,
.secondary-action {
    color: #d0d0d0;
    background-color: transparent;
    border-color: transparent;
}

.sidebar-chrome-button:hover,
.sidebar-icon-button:hover,
.sidebar-reopen-button:hover,
.sidebar-arrow-button:hover,
.chat-focus-btn:hover,
.chat-context-btn:hover,
.chat-toolbar-btn:hover,
.chat-mode-btn:hover,
.chat-mode-menu:hover,
.chat-editor-menu:hover,
.icon-button:hover,
.text-button:hover,
.flat-action:hover,
.secondary-action:hover,
.workspace-list row:hover,
.nav-button:hover,
.nav-row:hover,
.chat-menu-item:hover {
    background-color: @lc-hover;
    color: @lc-text-strong;
}

.nav-row-active,
.nav-button-active,
.workspace-list row:selected,
.chat-mode-selected,
.chat-menu-item-selected,
.project-tab-active {
    background-color: #2c2c2c;
    border-color: #5a5a5a;
    color: #f8fafc;
}

.kanban-board {
    background-color: #101010;
}

.kanban-column {
    background-color: #181818;
    border: 1px solid #2b2b2b;
    border-radius: 12px;
    box-shadow: none;
}

.kanban-column-header {
    border-bottom: 1px solid #2b2b2b;
}

.kanban-column .workspace-card {
    background-color: #151515;
    border-color: #303030;
    box-shadow: none;
}

.kanban-column .workspace-card:hover {
    background-color: #1d1d1d;
    border-color: #454545;
}

.suggested-action,
.chat-send-btn-active {
    background-color: #282828;
    border-color: #4a4a4a;
    color: #f4f4f4;
}

.suggested-action:hover,
.chat-send-btn-active:hover {
    background-color: #333333;
    border-color: #5a5a5a;
    color: #ffffff;
}

.destructive-action {
    background-color: #3f1d2a;
    border-color: #ef4444;
    color: #fecdd3;
}

entry,
entry text,
combobox,
combobox button,
combobox box,
.sidebar-search,
.composer-bar entry,
.workspace-modal-field,
.settings-machine-entry,
.settings-editor,
.settings-editor text {
    background-color: #0d0d0d;
    border-color: #343434;
    color: #f8fafc;
}

.chat-composer {
    background-color: transparent;
}

.chat-composer-box .chat-input-scroll,
.chat-composer-box .chat-input-view,
.chat-composer-box .chat-input-view text {
    background-color: transparent;
    border: none;
    box-shadow: none;
    color: #f8fafc;
}

entry,
combobox,
combobox button,
textview,
.workspace-modal-field,
.settings-machine-entry {
    border-radius: 8px;
    min-height: 40px;
}

entry placeholder,
textview placeholder {
    color: #737373;
}

entry:disabled,
combobox:disabled,
textview:disabled,
.workspace-modal-field:disabled,
.settings-machine-entry:disabled {
    background-color: #121212;
    border-color: #242424;
    color: #6f6f6f;
}

entry:focus,
.sidebar-search:focus,
.composer-bar entry:focus,
.workspace-modal-field:focus,
.settings-machine-entry:focus,
.settings-editor:focus,
combobox:focus,
combobox button:focus,
textview:focus,
.chat-composer-box:focus-within {
    border-color: #8a8a8a;
    box-shadow: 0 0 0 1px rgba(180, 180, 180, 0.34);
    outline: 1px solid transparent;
    outline-offset: 2px;
}

.chat-composer-box .chat-input-scroll:focus-within,
.chat-composer-box .chat-input-view:focus,
.chat-composer-box .chat-input-view text:focus {
    border: none;
    box-shadow: none;
    outline: none;
}

.workspace-modal-field:focus,
.settings-machine-entry:focus,
.settings-editor:focus,
.chat-composer-box:focus-within {
    border-color: #8a8a8a;
    box-shadow: 0 0 0 1px rgba(180, 180, 180, 0.34);
    outline-offset: 2px;
}

.chat-user-bubble {
    background-color: #2f2f2f;
    color: #f5f5f5;
}

.chat-inline-event-body {
    background-color: #0d0d0d;
    border-color: #2b2b2b;
    color: #d6d6d6;
}

.chat-inline-event-loading .chat-inline-event-chip {
    color: #c5d5f4;
}

.chat-inline-event-failed .chat-inline-event-chip {
    color: #f0a8a8;
}

.chat-context-usage-empty {
    background-color: #181818;
    border-color: #2b2b2b;
    color: #a3a3a3;
}

.chat-context-usage-normal {
    background-color: #181818;
    border-color: #2b2b2b;
    color: #a3a3a3;
}

.chat-context-usage-warning {
    background-color: #332b12;
    border-color: #f59e0b;
    color: #fde68a;
}

.chat-context-usage-danger {
    background-color: #3f1d2a;
    border-color: #ef4444;
    color: #fecdd3;
}

.chat-repo-icon,
.chat-editor-icon,
.chat-mode-icon,
.chat-menu-item-icon,
.chat-focus-btn image,
.chat-toolbar-btn image,
.sidebar-nav-icon {
    color: #93c5fd;
}

.workspace-row-branch-icon {
    color: @lc-text-muted;
}

.ws-check-icon-active,
.diff-added,
.status-running,
.workspace-status-running {
    color: #aaaaaa;
}

.diff-removed,
.status-error,
.workspace-status-error,
.ws-check-icon-fail {
    color: #fb7185;
}

.history-page-body {
    padding: 14px 20px 20px;
}

.history-filter-tabs {
    margin-bottom: 10px;
}

.history-split-pane {
    background-color: @lc-bg;
    border: 1px solid @lc-border;
    border-radius: 8px;
    box-shadow: none;
}

.history-split-pane separator {
    background-color: @lc-border;
    min-width: 1px;
}

.history-list row {
    margin: 0;
    border-radius: 0;
}

.history-list row:selected,
.history-list row:hover {
    background-color: @lc-hover-soft;
}

.history-list .history-row {
    background-color: transparent;
    padding: 10px 12px;
}

.history-detail {
    padding: 20px;
}

.history-transcript {
    background-color: @lc-bg;
    color: @lc-text;
    padding: 18px;
    border: none;
    border-radius: 0;
    box-shadow: none;
}
"#;

#[cfg(test)]
mod tests {
    use super::app_css;

    fn selector_block<'a>(css: &'a str, selector: &str) -> &'a str {
        let needle = format!("{selector} {{");
        let start = css.find(&needle).expect("selector exists in CSS");
        let rest = &css[start..];
        let end = rest.find("\n}").expect("selector block closes");
        &rest[..end]
    }

    #[test]
    fn refreshed_theme_exposes_graphite_palette_fonts_and_neutral_focus_color() {
        let css = app_css();

        assert!(css.contains("#101010"));
        assert!(css.contains("#1f1f1f"));
        assert!(css.contains("#333333"));
        assert!(css.contains("#5a5a5a"));
        assert!(css.contains("#8a8a8a"));
        assert!(css.contains("#aaaaaa"));
        assert!(css.contains("Mona Sans"));
        assert!(css.contains("Commit Mono"));
        assert!(css.contains(".workspace-modal-section"));
        assert!(css.contains(".setup-modal"));
        assert!(css.contains(".project-create-menu-row"));
        assert!(css.contains(".project-template-card"));
        assert!(css.contains("padding: 18px;"));
        assert!(css.contains("entry placeholder"));
        assert!(css.contains("combobox"));
        assert!(css.contains(".kanban-column-header"));
        assert!(css.contains(".dashboard-card-top"));
        assert!(css.contains("outline-offset: 2px"));
        assert!(css.contains(
            "transition-property: background-color, border-color, color, box-shadow, opacity"
        ));
        assert!(css.contains("transition-duration: 160ms"));
        assert!(css.contains(".chat-inline-event-chip"));
        let chip_block = selector_block(css, ".chat-inline-event-chip");
        assert!(chip_block.contains("background-color: transparent;"));
        assert!(chip_block.contains("border: none;"));
        assert!(chip_block.contains("font-size: 13px;"));
        assert!(chip_block.contains("min-height: 22px;"));
        assert!(chip_block.contains("padding: 2px 0;"));
        assert!(chip_block.contains("min-width: 0;"));
        assert!(css.contains("button.chat-inline-event-chip"));
        assert!(css.contains(".chat-inline-event-chip label"));
        assert!(css.contains("margin: 0;"));
        assert!(css.contains(".chat-reasoning-text"));
        assert!(css.contains(".chat-user-row {\n    margin-top: 12px;\n    margin-bottom: 10px;"));
        assert!(css.contains(".chat-agent-text {\n    color: #c6c6c6;\n    line-height: 1.55;\n    margin-bottom: 0;"));
        let inline_event_block = selector_block(css, ".chat-inline-event");
        assert!(inline_event_block.contains("background-color: transparent;"));
        assert!(inline_event_block.contains("margin-bottom: 0;"));
        assert!(!css.contains(".lc-accent-green .chat-send-btn-active"));
        assert!(!css.contains(".lc-accent-green .chat-user-bubble"));
        assert!(!css.contains(".lc-accent-green .suggested-action"));
        assert!(!css.contains("#38bdf8"));
        assert!(!css.contains("#2563eb"));
        assert!(!css.contains("#1d4ed8"));
        assert!(!css.contains("#eff6ff"));
        assert!(!css.contains("#0f172a"));
        assert!(!css.contains("#1e293b"));
        assert!(!css.contains("#334155"));
    }

    #[test]
    fn workspace_buttons_use_standard_app_font_stack_and_size() {
        let css = app_css();
        let text_button_block = selector_block(css, "button.text-button,\nbutton.icon-button");
        assert!(text_button_block.contains("font-family: \"Mona Sans\""));
        assert!(text_button_block.contains("font-size: 13px;"));

        let right_tab_block = selector_block(css, ".ws-right-tab-btn");
        assert!(right_tab_block.contains("font-family: \"Mona Sans\""));
        assert!(right_tab_block.contains("font-size: 13px;"));

        let changes_menu_block = selector_block(css, ".ws-changes-menu-btn,\n.ws-run-collapse-btn");
        assert!(changes_menu_block.contains("font-family: \"Mona Sans\""));
        assert!(changes_menu_block.contains("font-size: 13px;"));
    }

    #[test]
    fn queued_chat_overlay_floats_above_composer_with_hover_actions() {
        let css = app_css();
        let queue_overlay = selector_block(css, ".chat-queue-overlay");
        assert!(queue_overlay.contains("background-color: rgba(18, 18, 18, 0.96);"));
        assert!(queue_overlay.contains("margin: 0 8px 6px;"));

        let queued_row = selector_block(css, ".chat-queued-composer-row");
        assert!(queued_row.contains("background-color: #181818;"));
        assert!(queued_row.contains("border-radius: 6px;"));

        let queued_body = selector_block(css, ".chat-queued-composer-body");
        assert!(queued_body.contains("font-size: 13px;"));
        assert!(queued_body.contains("padding: 0;"));

        let actions = selector_block(css, ".chat-queued-actions");
        assert!(actions.contains("opacity: 0;"));
        assert!(css.contains(".chat-queued-composer-row:hover .chat-queued-actions"));

        let user_bubble = selector_block(css, ".chat-user-bubble");
        assert!(user_bubble.contains("background-color: #2e2e2e;"));
        assert!(!queued_row.contains("background-color: #2e2e2e;"));
    }

    #[test]
    fn workspace_view_preferences_do_not_target_sidebar_chrome() {
        let css = app_css();

        assert!(!css.contains("window.lc-theme-dark"));
        assert!(!css.contains(".lc-theme-dark .sidebar"));
        assert!(!css.contains(".lc-theme-light .sidebar"));
        assert!(!css.contains(".lc-theme-dark headerbar"));
        assert!(!css.contains(".lc-theme-light headerbar"));
        assert!(!css.contains(".lc-theme-dark .nav-button"));
        assert!(!css.contains(".lc-theme-light .nav-button"));
        assert!(!css.contains(".lc-theme-dark .nav-row"));
        assert!(!css.contains(".lc-theme-light .nav-row"));
        assert!(!css.contains(".lc-theme-dark .workspace-list"));
        assert!(!css.contains(".lc-theme-light .workspace-list"));
        assert!(!css.contains(".lc-theme-dark .sidebar-search"));
        assert!(!css.contains(".lc-theme-light .sidebar-search"));
        assert!(!css.contains(".lc-density-compact .sidebar"));
        assert!(!css.contains(".lc-density-comfortable .sidebar"));
        assert!(!css.contains(".lc-density-compact .nav-button"));
        assert!(!css.contains(".lc-density-compact .nav-row"));
        assert!(!css.contains(".lc-density-comfortable .nav-button"));
        assert!(!css.contains(".lc-density-comfortable .nav-row"));
    }

    #[test]
    fn dashboard_uses_flat_columns_and_actionable_cards() {
        let css = app_css();

        let board = selector_block(css, ".dashboard .page-board");
        assert!(board.contains("padding: 16px 20px;"));

        let column = selector_block(css, ".dashboard .kanban-column");
        assert!(column.contains("padding: 8px;"));
        assert!(column.contains("border: 1px solid"));

        let action = selector_block(css, ".workspace-card-action");
        assert!(action.contains("background: transparent;"));
        assert!(action.contains("border: none;"));
        assert!(action.contains("box-shadow: none;"));

        let card = selector_block(css, ".workspace-card-action .workspace-card");
        assert!(card.contains("border: 1px solid"));
        assert!(card.contains("box-shadow: none;"));
    }

    #[test]
    fn history_uses_one_flat_split_pane_border() {
        let css = app_css();

        let split = selector_block(css, ".history-split-pane");
        assert!(split.contains("border: 1px solid"));
        assert!(split.contains("box-shadow: none;"));

        let transcript = selector_block(css, ".history-transcript");
        assert!(transcript.contains("box-shadow: none;"));
    }

    #[test]
    fn app_css_styles_compact_variant_toasts() {
        let css = app_css();

        assert!(css.contains("toast {"));
        assert!(css.contains(".toast-content"));
        assert!(css.contains(".toast-info"));
        assert!(css.contains(".toast-success"));
        assert!(css.contains(".toast-warning"));
        assert!(css.contains(".toast-error"));
    }
}
