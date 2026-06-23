pub(crate) fn app_css() -> &'static str {
    APP_CSS
}

const APP_CSS: &str = r#"
window {
    background-color: #181a18;
    color: #e4e8e4;
    font-family: "Inter", "Cantarell", "Noto Sans", sans-serif;
}

.dashboard,
.page-shell,
.history-view {
    background-color: #181a18;
    color: #e4e8e4;
}

.page-header,
.dashboard-header {
    padding: 24px 30px 12px 30px;
    border-bottom: 1px solid #2a2c2a;
    background-color: #1c201d;
}

.page-body,
.detail-body,
.page-board,
.kanban-board {
    padding: 24px 30px;
}

.dashboard-title {
    color: #e4e8e4;
    font-size: 22px;
    font-weight: 600;
}

.section-title,
.sidebar-header,
.repo-section-header {
    color: #8a8f88;
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
    color: #8a8f88;
}

.detail-value,
.metric-value,
.column-title,
.card-title,
.workspace-name {
    color: #e4e8e4;
}

.sidebar {
    background-color: #1e201f;
    border-right: 1px solid #2a2c2a;
    padding-top: 12px;
}

.sidebar-nav-group {
    padding: 8px 10px 6px 10px;
}

.projects-header {
    padding: 16px 14px 8px 14px;
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
    color: #b4b8b4;
}

.nav-row-active,
.nav-button-active {
    color: #e4e8e4;
    background-color: #2c2f2c;
    border-color: transparent;
}

.nav-button:hover,
.nav-row:hover {
    color: #e4e8e4;
    background-color: #2a2e2c;
    border-color: transparent;
}

.sidebar-search,
.composer-bar entry,
entry {
    background-color: #181a18;
    color: #e4e8e4;
    border: 1px solid #2a2c2a;
    border-radius: 7px;
    font-size: 13px;
}

.sidebar-search:focus,
.composer-bar entry:focus,
entry:focus {
    border-color: #3fb950;
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
    background-color: #2c2f2c;
}

.workspace-list row:hover {
    background-color: #2a2e2c;
}

.workspace-row-shell,
.project-row,
.history-row {
    padding: 10px 12px;
}

.workspace-row-active {
    background-color: #2c2f2c;
    border: 1px solid transparent;
    border-radius: 7px;
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
    color: #8a8f88;
}

.project-icon-hot,
.card-diff-hot,
.run-dot-active,
.stat-running {
    color: #3fb950;
}

.app-header,
headerbar {
    background-color: #1c201d;
    border-bottom: 1px solid #2a2c2a;
}

.chrome-button {
    border-radius: 7px;
    background: transparent;
    border: 1px solid transparent;
}

button {
    background-image: none;
    background-color: transparent;
    color: #e4e8e4;
    border: 1px solid transparent;
    border-radius: 7px;
    box-shadow: none;
    text-shadow: none;
    padding: 7px 12px;
    min-height: 34px;
    font-weight: 500;
    letter-spacing: 0;
}

button:hover {
    background-color: rgba(255, 255, 255, 0.06);
    border-color: transparent;
}

button:active {
    background-color: rgba(255, 255, 255, 0.1);
}

button.suggested-action {
    background-color: #244c2e;
    color: #e4e8e4;
    border-color: transparent;
}

button.suggested-action:hover {
    background-color: #2d5c39;
    border-color: transparent;
}

button.secondary-action {
    background-color: #1e201f;
    color: #b4b8b4;
    border-color: #2a2c2a;
}

button.secondary-action:hover {
    background-color: #2a2e2c;
    color: #e4e8e4;
    border-color: #2a2c2a;
}

button.flat-action {
    background-image: none;
    background-color: transparent;
    color: #b4b8b4;
    border-color: transparent;
}

button.flat-action:hover {
    background-color: #2a2e2c;
    color: #e4e8e4;
    border-color: transparent;
}

button.destructive-action {
    background-color: #3a1f24;
    color: #e4e8e4;
    border-color: transparent;
}

button.destructive-action:hover {
    background-color: #4a252c;
    border-color: transparent;
}

checkbutton {
    color: #b4b8b4;
}

.chrome-button:hover {
    background-color: #2a2e2c;
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
    color: #3fb950;
}

.project-tab-active {
    border-bottom: 2px solid #3fb950;
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
    background-color: #1e201f;
    border: 1px solid #2a2c2a;
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
    color: #8a8f88;
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
    color: #8a8f88;
    border: 1px solid transparent;
    border-radius: 7px;
    padding: 6px 12px;
    font-size: 12px;
    font-weight: 600;
}

.panel-switcher button:hover {
    background-color: #2a2e2c;
    color: #e4e8e4;
    border-color: transparent;
}

.panel-switcher button:checked {
    background-color: #2c2f2c;
    color: #e4e8e4;
    border-color: transparent;
}

.terminal-panel,
.session-tool-surface,
.session-transcript,
.terminal-transcript-dark,
.checks-view,
.diff-view,
.status-container {
    background-color: #15181b;
    color: #e4e8e4;
    border: 1px solid #2a2c2a;
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
    background-color: #15181b;
    color: #e4e8e4;
    font-size: 12px;
    font-family: "JetBrains Mono", "Cascadia Code", monospace;
}

.session-surface .card-meta,
.session-tool-surface .card-meta,
.terminal-panel .card-meta {
    color: #8a8f88;
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
    background-color: #2a2e2c;
    color: #b4b8b4;
    border: 1px solid transparent;
    padding: 4px 9px;
    font-size: 12px;
    font-weight: 600;
}

.pill-button:hover {
    background-color: #2c2f2c;
}

.composer-bar {
    background-color: #1c201d;
    border-top: 1px solid #2a2c2a;
}

separator {
    background-color: #2a2c2a;
    min-width: 1px;
    min-height: 1px;
}

.mini-action-button {
    min-width: 28px;
    min-height: 28px;
    border-radius: 7px;
    background-color: transparent;
    color: #8a8f88;
    border: none;
    font-weight: 700;
}

.mini-action-button:hover {
    background-color: rgba(255, 255, 255, 0.06);
    color: #e4e8e4;
}

.repo-section-row {
    padding: 0;
}

.repo-section-icon,
.workspace-row-icon {
    color: #8a8f88;
}

.workspace-row-icon-active {
    color: #3fb950;
}

.repo-section-header {
    color: #c7cbc7;
    font-size: 13px;
    font-weight: 500;
    letter-spacing: 0;
    text-transform: none;
}

.repo-section-count {
    color: #8a8f88;
    font-size: 13px;
}

.repo-header-add {
    min-width: 24px;
    min-height: 24px;
    padding: 0;
    color: #8a8f88;
    border: none;
}

.repo-header-add:hover {
    color: #e4e8e4;
}

.settings-cta {
    background-color: #1e201f;
    border: 1px solid #2a2c2a;
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
    background-color: #1e201f;
    border: 1px solid #2a2c2a;
    border-radius: 12px;
    padding: 10px;
}

.toolbar-label {
    color: #8a8f88;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
}

.action-input-row {
    background-color: transparent;
}

.surface-note {
    color: #8a8f88;
    font-size: 12px;
}

.modal-body {
    background-color: #1c201d;
    border: 1px solid #2a2c2a;
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
}

.workspace-modal-hint {
    margin-top: 2px;
    margin-bottom: 2px;
}

.workspace-modal-preview {
    background-color: #15181b;
    border: 1px solid #2a2c2a;
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
    background-color: #1c201d;
    border: 1px solid #2a2c2a;
    border-radius: 14px;
    padding: 14px;
}

.settings-toolbar {
    background-color: #1c201d;
    border-bottom: 1px solid #2a2c2a;
    padding-bottom: 12px;
}

.settings-toolbar-row {
    margin: 0;
}

.settings-status {
    color: #8a8f88;
    font-size: 12px;
}

.settings-inspector {
    margin-top: 2px;
}

.settings-rail {
    background-color: #1e201f;
    border: 1px solid #2a2c2a;
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
    color: #b4b8b4;
}

.settings-rail-button:hover {
    background-color: #2a2e2c;
    border-color: transparent;
}

.settings-rail-button-active {
    color: #e4e8e4;
    background-color: #2c2f2c;
    border-color: transparent;
}

.settings-rail-title {
    color: #e4e8e4;
    font-size: 13px;
    font-weight: 600;
}

.settings-rail-copy {
    color: #8a8f88;
    font-size: 11px;
}

.settings-content-shell {
    background-color: #181a18;
    border: 1px solid #2a2c2a;
    border-radius: 14px;
    padding: 10px 12px;
}

.settings-content-panel {
    padding: 2px;
}

.settings-group {
    background-color: #1e201f;
    border: 1px solid #2a2c2a;
    border-radius: 14px;
    padding: 14px;
}

.settings-group-title {
    color: #e4e8e4;
    font-size: 14px;
    font-weight: 600;
}

.settings-group-copy {
    color: #8a8f88;
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
    background-color: #181a18;
    border: 1px solid #2a2c2a;
    border-radius: 10px;
    padding: 10px;
}

.settings-field-title {
    color: #e4e8e4;
    font-size: 13px;
    font-weight: 500;
}

.settings-field-copy {
    color: #8a8f88;
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
    background-color: #15181b;
    border: 1px solid #2a2c2a;
    border-radius: 10px;
}

.settings-editor {
    background-color: #15181b;
    color: #e4e8e4;
    padding: 8px;
}

.settings-shell entry,
.settings-shell combobox,
.settings-shell textview,
.settings-shell scrolledwindow {
    border-radius: 7px;
}

.settings-shell checkbutton {
    color: #e4e8e4;
}

.lc-accent-blue .section-title,
.lc-accent-blue .project-tab-active,
.lc-accent-blue .card-activity,
.lc-accent-blue .workspace-title {
    color: #2563eb;
    border-color: #2563eb;
}

.lc-accent-green .section-title,
.lc-accent-green .project-tab-active,
.lc-accent-green .card-activity,
.lc-accent-green .workspace-title {
    color: #3fb950;
    border-color: #3fb950;
}

.lc-accent-amber .section-title,
.lc-accent-amber .project-tab-active,
.lc-accent-amber .card-activity,
.lc-accent-amber .workspace-title {
    color: #b35c00;
    border-color: #b35c00;
}

.lc-accent-rose .section-title,
.lc-accent-rose .project-tab-active,
.lc-accent-rose .card-activity,
.lc-accent-rose .workspace-title {
    color: #be123c;
    border-color: #be123c;
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
    background-color: #181a18;
    color: #e4e8e4;
}

.lc-theme-dark .sidebar,
.lc-theme-dark .page-header,
.lc-theme-dark headerbar {
    background-color: #1c201d;
    border-color: #2a2c2a;
}

.lc-theme-dark .workspace-card,
.lc-theme-dark .command-panel,
.lc-theme-dark .metric-card,
.lc-theme-dark .detail-row,
.lc-theme-dark .settings-panel {
    background-color: #1e201f;
    border-color: #2a2c2a;
}

.lc-theme-dark .dashboard-title,
.lc-theme-dark .workspace-name,
.lc-theme-dark .card-title,
.lc-theme-dark .metric-value,
.lc-theme-dark .detail-value,
.lc-theme-dark .column-title {
    color: #e4e8e4;
}

.lc-theme-dark .card-meta,
.lc-theme-dark .workspace-meta,
.lc-theme-dark .detail-label,
.lc-theme-dark .project-tab,
.lc-theme-dark .column-count,
.lc-theme-dark .empty-label,
.lc-theme-dark .card-branch,
.lc-theme-dark .card-diff {
    color: #8a8f88;
}

.lc-theme-dark .nav-button,
.lc-theme-dark .nav-row {
    color: #b4b8b4;
}

.lc-theme-dark .nav-button-active,
.lc-theme-dark .nav-row-active,
.lc-theme-dark .nav-button:hover,
.lc-theme-dark .workspace-list row:selected,
.lc-theme-dark .workspace-list row:hover {
    background-color: #2c2f2c;
    color: #e4e8e4;
    border-color: transparent;
}

.lc-theme-dark .sidebar-search,
.lc-theme-dark .composer-bar entry,
.lc-theme-dark entry {
    background-color: #181a18;
    color: #e4e8e4;
    border-color: #2a2c2a;
}

.lc-theme-dark .chrome-button:hover,
.lc-theme-dark .panel-switcher button:hover,
.lc-theme-dark .panel-switcher button:checked {
    background-color: #2c2f2c;
    border-color: transparent;
    color: #e4e8e4;
}

.lc-theme-dark .settings-shell {
    background-color: #1c201d;
    border-color: #2a2c2a;
}

.lc-theme-dark .settings-toolbar,
.lc-theme-dark .settings-content-shell,
.lc-theme-dark .settings-group,
.lc-theme-dark .settings-rail {
    border-color: #2a2c2a;
}

/* ── Sidebar status dots ── */
.status-dot {
    min-width: 8px;
    min-height: 8px;
    border-radius: 50%;
    background-color: #8a8f88;
    opacity: 0.5;
    margin-top: 5px;
}
.status-dot-active {
    background-color: #3fb950;
    opacity: 1.0;
}
.status-dot-running {
    background-color: #3fb950;
    opacity: 1.0;
}
.status-dot-idle {
    background-color: #8a8f88;
    opacity: 0.45;
}

/* ── Sidebar row timestamp ── */
.workspace-row-timestamp {
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
    font-size: 11px;
    color: #8a8f88;
}

/* ── Workspace ahead-commits badge ── */
.workspace-badge {
    font-family: "JetBrains Mono", "Cascadia Mono", monospace;
    font-size: 11px;
    font-weight: 600;
    color: #3fb950;
    background-color: rgba(63, 185, 80, 0.12);
    border-radius: 7px;
    padding: 1px 6px;
}
.workspace-badge-muted {
    color: #b4b8b4;
    background-color: transparent;
}

/* ── Sidebar bottom bar ── */
.sidebar-bottom-bar {
    background-color: #1c201d;
    border-top: 1px solid #2a2c2a;
    padding: 6px 8px;
    min-height: 44px;
}
.sidebar-bottom-icon-btn {
    background: transparent;
    border: none;
    border-radius: 7px;
    padding: 5px;
    color: #8a8f88;
    min-width: 28px;
    min-height: 28px;
}
.sidebar-bottom-icon-btn:hover {
    background-color: #2a2e2c;
    color: #e4e8e4;
}
.sidebar-bottom-icon-btn.active {
    color: #e4e8e4;
}

/* ── Section header ── */
.repo-section-row {
    padding: 10px 12px 4px 12px;
}
.repo-section-header {
    font-size: 12px;
    font-weight: 600;
    color: #e4e8e4;
    letter-spacing: 0;
}
.repo-section-chevron {
    font-size: 11px;
    color: #8a8f88;
}

/* ── Updated workspace row shell ── */
.workspace-row-shell {
    padding: 7px 10px 7px 12px;
}

/* ── Minimal sidebar search ── */
.sidebar-search-minimal {
    background: transparent;
    border: none;
    border-bottom: 1px solid #2a2c2a;
    border-radius: 0;
    font-size: 12px;
    color: #8a8f88;
    padding: 6px 12px;
    margin: 0;
}
.sidebar-search-minimal:focus {
    border-bottom-color: #3fb950;
    color: #e4e8e4;
}
"#;
