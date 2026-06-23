pub(crate) fn app_css() -> &'static str {
    APP_CSS
}

const APP_CSS: &str = r#"
window {
    background-color: #191919;
    color: #e4e4e4;
    font-family: "Inter", "Cantarell", "Noto Sans", sans-serif;
}

.dashboard,
.page-shell,
.history-view {
    background-color: #191919;
    color: #e4e4e4;
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
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
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
    background-color: #202020;
    border-right: 1px solid #2a2a2a;
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
    padding: 12px 10px 6px 10px;
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
    background-color: #191919;
    color: #e4e4e4;
    border: 1px solid #2a2a2a;
    border-radius: 7px;
    font-size: 13px;
}

.sidebar-search:focus,
.composer-bar entry:focus,
entry:focus {
    border-color: #aaaaaa;
    box-shadow: none;
}

.workspace-list {
    background-color: transparent;
}

.workspace-list row {
    border-radius: 7px;
    margin: 2px 10px;
    padding: 0;
}

.workspace-list row:selected {
    background-color: #2e2e2e;
}

.workspace-list row:hover {
    background-color: #2c2c2c;
}

.workspace-row-shell,
.project-row,
.history-row {
    padding: 8px 10px;
}

.workspace-row-active {
    background-color: #2e2e2e;
    border: 1px solid transparent;
    border-radius: 7px;
}

.workspace-row-branch-icon,
.workspace-row-branch-icon-active {
    min-width: 14px;
    color: #7e7e7e;
}

.workspace-row-branch-icon-active {
    color: #aaaaaa;
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
    color: #7fb6ff;
    background-color: transparent;
    border: none;
}

.workspace-status-chip-active {
    color: #7fb6ff;
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
    padding-bottom: 10px;
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
    min-width: 240px;
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
    min-height: 116px;
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
    font-family: "JetBrains Mono", "Cascadia Code", monospace;
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
    color: #8a8a8a;
}

.workspace-row-icon-active {
    color: #aaaaaa;
}

.repo-section-header {
    color: #c8c8c8;
    font-size: 13px;
    font-weight: 500;
    letter-spacing: 0;
    text-transform: none;
}

.repo-section-count {
    color: #8a8a8a;
    font-size: 13px;
}

.repo-header-add {
    min-width: 24px;
    min-height: 24px;
    padding: 0;
    color: #8a8a8a;
    border: none;
}

.repo-header-add:hover {
    color: #e4e4e4;
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
    padding: 2px;
}

.workspace-modal {
    padding: 8px;
}

.workspace-modal-split {
    margin-top: 4px;
}

.workspace-modal-field {
    min-height: 40px;
    min-width: 0;
}

.workspace-modal-hint {
    margin-top: 2px;
    margin-bottom: 2px;
}

.workspace-modal-preview {
    background-color: #151515;
    border: 1px solid #2a2a2a;
    border-radius: 12px;
    padding: 12px;
    margin-top: 4px;
}

.workspace-modal-preview-copy {
    line-height: 1.45;
}

.workspace-modal-feedback {
    margin-top: 2px;
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
    background-color: #191919;
    border: 1px solid #2a2a2a;
    border-radius: 14px;
    padding: 10px 12px;
}

.settings-content-panel {
    padding: 2px;
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
    background-color: #191919;
    border: 1px solid #2a2a2a;
    border-radius: 10px;
    padding: 10px;
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

.settings-machine-entry,
.settings-editor,
.settings-editor text,
.settings-editor view,
.settings-editor widget {
    font-family: "JetBrains Mono", "Cascadia Code", monospace;
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
.lc-accent-blue .workspace-title,
.lc-accent-blue .chat-mode-selected,
.lc-accent-blue .chat-send-btn-active,
.lc-accent-blue .chat-user-bubble {
    color: #eff6ff;
    border-color: #2563eb;
}
.lc-accent-blue .chat-mode-selected,
.lc-accent-blue .chat-send-btn-active,
.lc-accent-blue .chat-user-bubble {
    background-color: #2563eb;
}

.lc-accent-green .section-title,
.lc-accent-green .project-tab-active,
.lc-accent-green .card-activity,
.lc-accent-green .workspace-title,
.lc-accent-green .chat-mode-selected,
.lc-accent-green .chat-send-btn-active,
.lc-accent-green .chat-user-bubble {
    color: #ecfdf5;
    border-color: #22c55e;
}
.lc-accent-green .chat-mode-selected,
.lc-accent-green .chat-send-btn-active,
.lc-accent-green .chat-user-bubble {
    background-color: #22c55e;
}

.lc-accent-amber .section-title,
.lc-accent-amber .project-tab-active,
.lc-accent-amber .card-activity,
.lc-accent-amber .workspace-title,
.lc-accent-amber .chat-mode-selected,
.lc-accent-amber .chat-send-btn-active,
.lc-accent-amber .chat-user-bubble {
    color: #fff7ed;
    border-color: #b35c00;
}
.lc-accent-amber .chat-mode-selected,
.lc-accent-amber .chat-send-btn-active,
.lc-accent-amber .chat-user-bubble {
    background-color: #b35c00;
}

.lc-accent-rose .section-title,
.lc-accent-rose .project-tab-active,
.lc-accent-rose .card-activity,
.lc-accent-rose .workspace-title,
.lc-accent-rose .chat-mode-selected,
.lc-accent-rose .chat-send-btn-active,
.lc-accent-rose .chat-user-bubble {
    color: #fff1f2;
    border-color: #be123c;
}
.lc-accent-rose .chat-mode-selected,
.lc-accent-rose .chat-send-btn-active,
.lc-accent-rose .chat-user-bubble {
    background-color: #be123c;
}

.lc-density-compact .nav-row,
.lc-density-compact .nav-row-active,
.lc-density-compact .nav-button,
.lc-density-compact .nav-button-active {
    padding: 8px 10px;
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

.lc-density-comfortable .nav-row,
.lc-density-comfortable .nav-row-active,
.lc-density-comfortable .nav-button,
.lc-density-comfortable .nav-button-active {
    padding: 12px 16px;
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

window.lc-theme-dark,
.lc-theme-dark .dashboard,
.lc-theme-dark .page-shell,
.lc-theme-dark .history-view {
    background-color: #191919;
    color: #e4e4e4;
}

.lc-theme-dark .sidebar,
.lc-theme-dark .page-header,
.lc-theme-dark headerbar {
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

.lc-theme-dark .nav-button,
.lc-theme-dark .nav-row {
    color: #b4b4b4;
}

.lc-theme-dark .nav-button-active,
.lc-theme-dark .nav-row-active,
.lc-theme-dark .nav-button:hover,
.lc-theme-dark .workspace-list row:selected,
.lc-theme-dark .workspace-list row:hover {
    background-color: #2e2e2e;
    color: #e4e4e4;
    border-color: transparent;
}

.lc-theme-dark .sidebar-search,
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
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
    font-size: 11px;
    color: #8a8a8a;
}

/* ── Workspace ahead-commits badge ── */
.workspace-badge {
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
    font-size: 11px;
    font-weight: 600;
    color: #aaaaaa;
    background-color: rgba(255, 255, 255, 0.08);
    border-radius: 7px;
    padding: 1px 6px;
}
.workspace-badge-muted {
    color: #b4b4b4;
    background-color: transparent;
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
    padding: 8px 10px 4px 10px;
}
.repo-section-header {
    font-size: 12px;
    font-weight: 600;
    color: #e4e4e4;
    letter-spacing: 0;
}
.repo-section-icon {
    min-width: 14px;
    color: #8a8a8a;
}

/* ── Updated workspace row shell ── */
.workspace-row-shell {
    padding: 7px 10px 7px 10px;
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
.ws-tab-sep {
    min-height: 1px;
    background-color: #282828;
}
.ws-tab-btn {
    background: transparent;
    border: none;
    border-radius: 0;
    border-bottom: 2px solid transparent;
    padding: 0 4px;
    margin: 0;
    font-size: 14px;
    font-weight: 400;
    color: #8a8a8a;
    min-height: 46px;
}
.ws-tab-btn:hover {
    background: transparent;
    color: #c6c6c6;
}
.ws-tab-active {
    color: #e8e8e8;
    border-bottom-color: #e8e8e8;
    font-weight: 600;
}
.ws-mode-switcher button {
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
.ws-pr-number {
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
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
    background-color: #163522;
    color: #84e0a0;
}
.ws-pr-status-failed {
    background-color: #3a1a1a;
    color: #ff8a8a;
}
.ws-pr-status-merged {
    background-color: #311d46;
    color: #c6a3ff;
}
.ws-changes-menu-btn,
.ws-run-collapse-btn {
    background: transparent;
    border: none;
    box-shadow: none;
    color: #8e8e8e;
    font-size: 16px;
    min-width: 30px;
    min-height: 30px;
    border-radius: 6px;
}
.ws-changes-menu-btn:hover,
.ws-run-collapse-btn:hover {
    background-color: #242424;
    color: #e4e4e4;
}

/* ── File list ── */
.ws-file-list {
    background: transparent;
}
.ws-dir-row {
    padding: 5px 11px 2px;
}
.ws-folder-icon {
    font-size: 10px;
    color: #888888;
    min-width: 14px;
}
.ws-folder-name {
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
    font-size: 12px;
    font-weight: 600;
    color: #909090;
}
.ws-file-row {
    padding: 5px 11px;
}
.ws-file-badge {
    font-family: "JetBrains Mono", monospace;
    font-size: 10px;
    font-weight: 600;
    color: #5d5d5d;
    min-width: 18px;
}
.ws-file-name {
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
    font-size: 13px;
    color: #b2b2b2;
}
.ws-file-dir {
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
    font-size: 11px;
    color: #5d5d5d;
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
    font-family: "JetBrains Mono", "Cascadia Mono", "Fira Code", monospace;
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
.chat-menu-list {
    padding: 8px;
}
.chat-menu-item {
    background: transparent;
    border: none;
    box-shadow: none;
    text-shadow: none;
    border-radius: 10px;
    padding: 9px 10px;
    color: #d8d8d8;
}
.chat-menu-item:hover,
.chat-menu-item-selected {
    background-color: #2a2a2a;
}
.chat-menu-item-label {
    color: inherit;
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
.chat-messages {
    padding: 22px 24px 8px;
}
.chat-user-row {
    margin-bottom: 22px;
}
.chat-user-bubble {
    background-color: #2e2e2e;
    color: #f4f7ff;
    border-radius: 14px;
    padding: 11px 16px;
    line-height: 1.4;
    min-height: 42px;
}
.chat-agent-text {
    color: #c6c6c6;
    line-height: 1.55;
    margin-bottom: 18px;
}
.chat-composer {
    padding: 0 16px 16px;
    background-color: #151515;
}
.chat-composer-box {
    border: 1px solid #2a2a2a;
    border-radius: 14px;
    background-color: #202020;
}
.chat-placeholder {
    color: #7f7f7f;
    font-size: 14px;
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
    font-size: 14px;
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
    background-color: #2563eb;
    color: #eff6ff;
}
.chat-send-btn:hover {
    background-color: #383838;
    color: #e4e4e4;
}
.chat-send-btn-active:hover {
    background-color: #1d4ed8;
    color: #eff6ff;
}

/* ── Diff view ── */
.ws-diff-view,
.ws-diff-view text {
    background-color: #131313;
    color: #c0c0c0;
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
    font-size: 12px;
    line-height: 1.5;
}

/* ── Center panel file code view ── */
.ws-file-code-view,
.ws-file-code-view text {
    background-color: #161616;
    color: #c8c8c8;
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
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
    font-family: "JetBrains Mono", monospace;
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
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
    font-size: 12px;
    color: #c0c0c0;
    padding-left: 8px;
}
.ws-check-sub {
    font-size: 11px;
    color: #666666;
    font-family: "JetBrains Mono", monospace;
    padding-top: 1px;
    padding-bottom: 1px;
}
"#;
