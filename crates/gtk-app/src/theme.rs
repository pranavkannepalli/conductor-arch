pub(crate) fn app_css() -> &'static str {
    APP_CSS
}

const APP_CSS: &str = r#"
window {
    background-color: #f3f4ef;
    color: #101828;
}

.dashboard,
.page-shell,
.history-view {
    background-color: #f3f4ef;
    color: #101828;
}

.page-header,
.dashboard-header {
    padding: 24px 30px 12px 30px;
    border-bottom: 1px solid #dde1d7;
    background-color: #f7f7f2;
}

.page-body,
.detail-body,
.page-board,
.kanban-board {
    padding: 24px 30px;
}

.dashboard-title {
    color: #101828;
    font-size: 22px;
    font-weight: 700;
}

.section-title,
.sidebar-header,
.repo-section-header {
    color: #667085;
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
}

.section-title {
    margin-top: 8px;
}

.card-meta,
.workspace-meta,
.detail-label,
.project-tab,
.column-count,
.workspace-path-label {
    color: #667085;
}

.detail-value,
.metric-value,
.column-title,
.card-title,
.workspace-name {
    color: #101828;
}

.sidebar {
    background-color: #f7f7f2;
    border-right: 1px solid #d9dcd1;
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
    padding: 10px 12px;
    border-radius: 10px;
    background: transparent;
    border: 1px solid transparent;
    box-shadow: none;
    text-shadow: none;
    font-size: 14px;
    font-weight: 500;
}

.nav-row,
.nav-button {
    color: #475467;
}

.nav-row-active,
.nav-button-active {
    color: #101828;
    background-color: #ebeee5;
    border-color: #d9dcd1;
}

.nav-button:hover,
.nav-row:hover {
    color: #101828;
    background-color: #f0f2ea;
    border-color: #e1e5da;
}

.sidebar-search,
.composer-bar entry,
entry {
    background-color: #ffffff;
    color: #101828;
    border: 1px solid #d0d5dd;
    border-radius: 10px;
    font-size: 13px;
}

.sidebar-search:focus,
.composer-bar entry:focus,
entry:focus {
    border-color: #7aa26a;
    box-shadow: none;
}

.workspace-list {
    background-color: transparent;
}

.workspace-list row {
    border-radius: 10px;
    margin: 2px 10px;
    padding: 0;
}

.workspace-list row:selected {
    background-color: #ebeee5;
}

.workspace-list row:hover {
    background-color: #f0f2ea;
}

.workspace-row-shell,
.project-row,
.history-row {
    padding: 10px 12px;
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

.project-icon {
    color: #98a2b3;
}

.project-icon-hot,
.card-diff-hot,
.run-dot-active,
.stat-running {
    color: #4f8a3f;
}

.app-header,
headerbar {
    background-color: #f7f7f2;
    border-bottom: 1px solid #dde1d7;
}

.chrome-button {
    border-radius: 10px;
    background: transparent;
    border: 1px solid transparent;
}

.chrome-button:hover {
    background-color: #eceee6;
    border-color: #d9dcd1;
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
    color: #3f6f35;
}

.project-tab-active {
    border-bottom: 2px solid #3f6f35;
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
    background-color: #fbfbf8;
    border: 1px solid #d9dcd1;
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
    color: #667085;
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
    color: #667085;
    border: 1px solid transparent;
    border-radius: 10px;
    padding: 6px 12px;
    font-size: 12px;
    font-weight: 600;
}

.panel-switcher button:hover {
    background-color: #eef2e8;
    color: #101828;
    border-color: #d9dcd1;
}

.panel-switcher button:checked {
    background-color: #eef2e8;
    color: #101828;
    border-color: #d9dcd1;
}

.terminal-panel,
.session-tool-surface,
.session-transcript,
.terminal-transcript-dark,
.checks-view,
.diff-view,
.status-container {
    background-color: #101418;
    color: #d0d5dd;
    border: 1px solid #27303a;
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
    background-color: #101418;
    color: #d0d5dd;
    font-size: 12px;
    font-family: "JetBrains Mono", "Fira Code", "Cascadia Code", monospace;
}

.session-surface .card-meta,
.session-tool-surface .card-meta,
.terminal-panel .card-meta {
    color: #98a2b3;
}

.terminal-tab-strip button,
.pill-button {
    border-radius: 8px;
}

.terminal-tab-strip button {
    font-size: 12px;
    padding: 5px 10px;
}

.pill-button {
    background-color: #1a222a;
    color: #d0d5dd;
    border: 1px solid #34414f;
    padding: 2px 10px;
    font-size: 12px;
}

.pill-button:hover {
    background-color: #24303b;
}

.composer-bar {
    background-color: #f7f7f2;
    border-top: 1px solid #dde1d7;
}

separator {
    background-color: #dde1d7;
    min-width: 1px;
    min-height: 1px;
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
    color: #3f6f35;
    border-color: #3f6f35;
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
    background-color: #15191e;
    color: #e5e7eb;
}

.lc-theme-dark .sidebar,
.lc-theme-dark .page-header,
.lc-theme-dark headerbar {
    background-color: #171c22;
    border-color: #28313a;
}

.lc-theme-dark .workspace-card,
.lc-theme-dark .command-panel,
.lc-theme-dark .metric-card,
.lc-theme-dark .detail-row,
.lc-theme-dark .settings-panel {
    background-color: #1b2128;
    border-color: #2d3742;
}

.lc-theme-dark .dashboard-title,
.lc-theme-dark .workspace-name,
.lc-theme-dark .card-title,
.lc-theme-dark .metric-value,
.lc-theme-dark .detail-value,
.lc-theme-dark .column-title {
    color: #f3f4f6;
}

.lc-theme-dark .card-meta,
.lc-theme-dark .workspace-meta,
.lc-theme-dark .detail-label,
.lc-theme-dark .project-tab,
.lc-theme-dark .column-count,
.lc-theme-dark .empty-label,
.lc-theme-dark .card-branch,
.lc-theme-dark .card-diff {
    color: #98a2b3;
}

.lc-theme-dark .nav-button,
.lc-theme-dark .nav-row {
    color: #c7ced6;
}

.lc-theme-dark .nav-button-active,
.lc-theme-dark .nav-row-active,
.lc-theme-dark .nav-button:hover,
.lc-theme-dark .workspace-list row:selected,
.lc-theme-dark .workspace-list row:hover {
    background-color: #24303b;
    color: #f9fafb;
    border-color: #34414f;
}

.lc-theme-dark .sidebar-search,
.lc-theme-dark .composer-bar entry,
.lc-theme-dark entry {
    background-color: #11161c;
    color: #e5e7eb;
    border-color: #34414f;
}

.lc-theme-dark .chrome-button:hover,
.lc-theme-dark .panel-switcher button:hover,
.lc-theme-dark .panel-switcher button:checked {
    background-color: #24303b;
    border-color: #34414f;
    color: #f3f4f6;
}
"#;
