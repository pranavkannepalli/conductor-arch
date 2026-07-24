# GTK Refresh Refactor Map

This document maps the GTK refresh refactor in commit `11a6ebc`.

It covers GTK refresh state only: `RefreshScope`, `WorkspaceRefreshTarget`,
`RefreshEvent`, registered refresh slots, metrics, and migrated call sites. It
does not describe durable workspace/session/chat state in SQLite.

## Goal

The old refresh model let routine changes rebuild large UI surfaces:

- whole app through `RefreshScope::All`
- whole workspace through `RefreshScope::Workspace` and
  `WorkspaceRefreshTarget::Shell`
- whole workspace shell fallback through `run_event_or_shell`
- sidebar/dashboard/history refreshes for runtime, chat, and review child
  changes
- right-panel refreshes through whole workspace rebuilds

The new model keeps `RefreshHub` as a small event router. Routine events should
update the smallest mounted element that owns the changed data. Mount rebuilds
remain only for selection, inventory, lifecycle, settings, and explicit debug
recovery.

## Big Handle Breakdown

`RefreshScope::All`

- `RefreshHub::debug_full_refresh()`

`RefreshScope::Workspace`

- `RefreshHub::set_workspace_mount()`
- `RefreshHub::refresh_workspace_mount()`
- `RefreshEvent::WorkspaceSelectionChanged`
- `RefreshEvent::WorkspaceInventoryChanged`
- `RefreshEvent::WorkspaceLifecycleChanged`
- `RefreshEvent::SettingsChanged`

`WorkspaceRefreshTarget::Shell`

- `RefreshHub::set_workspace_mount()`
- `RefreshHub::refresh_workspace_mount()`

`run_event_or_shell`

- `run_event` for chat surface
- `run_event` for chat tabs
- `run_event` for runtime
- `run_event` for review
- no shell fallback when a small handler is missing

`RefreshEvent::Manual`

- `RefreshHub::debug_full_refresh()`

`RefreshEvent::WorkspaceMetadataChanged`

- `RefreshEvent::WorkspaceHeaderChanged { workspace }`
- `RefreshEvent::WorkspaceStatusChanged { workspace }`
- `RefreshEvent::WorkspaceDiffStatsChanged { workspace, additions, deletions }`
- `RefreshEvent::WorkspaceBranchChanged { workspace }`
- `RefreshEvent::WorkspaceLifecycleChanged { workspace }`
- `RefreshEvent::WorkspaceMetadataChanged { old_workspace, workspace, branch }`

`RefreshEvent::WorkspaceRuntimeChanged`

- `RefreshEvent::WorkspaceRuntimeChanged { workspace }`
- `RefreshEvent::RuntimeProcessChanged { workspace, process_id }`

`RefreshEvent::WorkspaceReviewChanged`

- `RefreshEvent::WorkspaceReviewChanged { workspace }`
- `RefreshEvent::ReviewCommentsChanged { workspace }`

`RefreshEvent::WorkspaceGitReviewChanged`

- `RefreshEvent::WorkspaceGitReviewChanged { workspace }`
- workspace nav row handler
- workspace review handler

`RefreshEvent::WorkspaceChatLifecycleChanged`

- `RefreshEvent::WorkspaceChatLifecycleChanged { workspace }`
- `RefreshEvent::ChatTabChanged { workspace, thread_id }`
- `RefreshEvent::ChatSessionStatusChanged { workspace, thread_id, session_id }`

`RefreshEvent::WorkspaceChatMessagesChanged`

- `RefreshEvent::WorkspaceChatMessagesChanged { workspace, thread_id }`
- `RefreshEvent::ChatMessageAppended { workspace, thread_id, message_id }`
- `RefreshEvent::ChatMessageUpdated { workspace, thread_id, message_id }`
- `RefreshEvent::ChatTimelineTailChanged { workspace, thread_id }`

`RefreshEvent::TerminalChanged`

- `RefreshEvent::TerminalChanged { workspace }`
- `RefreshEvent::TerminalBufferChanged { workspace, terminal_id }`
- `RefreshEvent::RuntimeProcessChanged { workspace, process_id }`

Whole sidebar/dashboard/history refresh for runtime/chat/review child changes

- workspace nav row handler
- sidebar row additions label
- sidebar row deletions label
- future dashboard card handlers
- future history row handlers
- future runtime badge/count handlers
- future chat badge/count handlers
- future review badge/count handlers

Whole right-panel refresh for child changes

- `RefreshEvent::RightPanelFileListChanged { workspace }`
- `RefreshEvent::RightPanelSelectedFileChanged { workspace, path }`
- `RefreshEvent::RightPanelDiffPreviewChanged { workspace, path }`
- `set_right_panel_file_list`
- `set_right_panel_diff_preview`

Whole chat surface refresh for child changes

- chat message append
- chat message update
- chat timeline tail change
- chat composer change
- chat queue change
- chat tab change
- chat session status change

Other broad or not-yet-split panel refreshes

- `RefreshEvent::ReviewCommentsChanged { workspace }`
- `RefreshEvent::TodosChanged { workspace }`
- `RefreshEvent::TerminalBufferChanged { workspace, terminal_id }`
- `RefreshEvent::RuntimeProcessChanged { workspace, process_id }`
- `RefreshEvent::SettingsSectionChanged { scope, section }`

## Files Changed

| File | Change |
| --- | --- |
| `crates/gtk-app/src/refresh.rs` | Removed broad refresh states, removed shell fallback, added small events, added right-panel child slots, added guard tests. |
| `crates/gtk-app/src/main.rs` | Replaced routine full refresh with debug full refresh; replaced command-palette terminal refresh with `TerminalChanged`. |
| `crates/gtk-app/src/sidebar.rs` | Added row-level refs for additions/deletions labels and updates them from `WorkspaceDiffStatsChanged`. |
| `crates/gtk-app/src/workspace_command_center.rs` | Replaced several workspace-shell rebuild callers with targeted workspace/right-panel events; registered right-panel file-list and diff-preview child handlers. |

## Status Legend

| Status | Meaning |
| --- | --- |
| Preserved | The old state still exists and still has the same broad meaning. |
| Split | The old state was broken into smaller events or handlers. |
| Mount-only | The old broad rebuild is still callable, but only as a mounted page/shell replacement path. |
| Debug-only | The behavior exists only behind `ARCHDUCTOR_GTK_DEBUG_FULL_REFRESH`. |
| Placeholder | The event exists so callers can stop using broad refreshes, but no mounted child handler is wired yet. |
| Removed | The old state/API no longer exists. |

## Old `RefreshScope` Breakdown

Before:

```rust
pub enum RefreshScope {
    All,
    Sidebar,
    Dashboard,
    Projects,
    History,
    Workspace,
}
```

After:

```rust
pub enum RefreshScope {
    Sidebar,
    Dashboard,
    Projects,
    History,
}
```

| Old state | Old fanout | New state(s) | Status | Notes |
| --- | --- | --- | --- | --- |
| `RefreshScope::All` | Sidebar, dashboard, projects, history, workspace shell. | `RefreshHub::debug_full_refresh()` | Debug-only | It only runs when `ARCHDUCTOR_GTK_DEBUG_FULL_REFRESH` is enabled. Keyboard refresh and command-palette refresh now call this debug path. Routine code is guarded against `RefreshScope::All`. |
| `RefreshScope::Sidebar` | Sidebar page/list callback. | `RefreshScope::Sidebar` | Preserved | Still used for page/navigation and inventory-level changes. It should not be used for row-level runtime/chat/diff changes. |
| `RefreshScope::Dashboard` | Dashboard panel callback. | `RefreshScope::Dashboard` | Preserved | Still used for project/workspace inventory changes. Routine child changes no longer fan out here from runtime/chat/review events. |
| `RefreshScope::Projects` | Projects/settings callback. | `RefreshScope::Projects` | Preserved | Still used for project inventory and settings-level changes. |
| `RefreshScope::History` | History page callback. | `RefreshScope::History` | Preserved | Still used for workspace inventory/lifecycle changes. Routine runtime/chat/review events no longer refresh history. |
| `RefreshScope::Workspace` | Whole workspace shell callback. | `RefreshHub::set_workspace_mount()` and `RefreshHub::refresh_workspace_mount()` | Mount-only | The enum variant is removed. The mount callback remains for selected workspace mount changes, inventory/lifecycle changes, and settings changes. |

## Old `WorkspaceRefreshTarget` Breakdown

Before:

```rust
pub enum WorkspaceRefreshTarget {
    Shell,
    ChatSurface,
    ChatTabs,
    Runtime,
    Review,
}
```

After:

```rust
enum WorkspaceRefreshTarget {
    ChatSurface,
    ChatTabs,
    Runtime,
    Review,
}
```

| Old state | Old behavior | New state(s) | Status | Notes |
| --- | --- | --- | --- | --- |
| `WorkspaceRefreshTarget::Shell` | Ran the whole workspace shell callback. | `RefreshHub::refresh_workspace_mount()` | Mount-only | The enum state is removed. Direct callers cannot request shell as a routine target. |
| `WorkspaceRefreshTarget::ChatSurface` | Ran chat surface handler with no shell fallback. | `WorkspaceRefreshTarget::ChatSurface` plus chat message events | Split | Still internal. It is reached by `WorkspaceChatMessagesChanged`, `ChatMessageAppended`, `ChatMessageUpdated`, and `ChatTimelineTailChanged`. |
| `WorkspaceRefreshTarget::ChatTabs` | Ran chat-tabs handler, or rebuilt workspace shell when missing. | `WorkspaceRefreshTarget::ChatTabs` | Split | No fallback. Currently reached by `WorkspaceChatLifecycleChanged`. More-specific chat tab/status events exist but are placeholders until child handlers are wired. |
| `WorkspaceRefreshTarget::Runtime` | Ran runtime handler, or rebuilt workspace shell when missing. | `WorkspaceRefreshTarget::Runtime` | Split | No fallback. Currently reached by `WorkspaceRuntimeChanged` and `TerminalChanged`. More-specific runtime/terminal events exist as placeholders. |
| `WorkspaceRefreshTarget::Review` | Ran review handler, or rebuilt workspace shell when missing. | `WorkspaceRefreshTarget::Review` | Split | No fallback. Currently reached by `WorkspaceReviewChanged` and `WorkspaceGitReviewChanged`. `ReviewCommentsChanged` exists as a placeholder. |

## Removed Fallback

Before, `run_event_or_shell` did this:

1. try the targeted handler
2. if no handler is registered, run the workspace shell callback

That meant a missing small handler silently became a whole workspace rebuild.

After, the fallback is gone. Missing targeted handlers do nothing. Tests now
assert this for runtime, review, chat lifecycle, and chat message events.

## Old `RefreshEvent` Breakdown

Before:

```rust
pub enum RefreshEvent {
    Manual,
    ProjectInventoryChanged,
    SettingsChanged,
    WorkspaceSelectionChanged,
    WorkspaceInventoryChanged,
    WorkspaceMetadataChanged { old_workspace, workspace, branch },
    WorkspaceRuntimeChanged { workspace },
    WorkspaceReviewChanged { workspace },
    WorkspaceGitReviewChanged { workspace },
    WorkspaceChatLifecycleChanged { workspace },
    WorkspaceChatMessagesChanged { workspace, thread_id },
    TerminalChanged { workspace },
}
```

After:

```rust
pub enum RefreshEvent {
    ProjectInventoryChanged,
    SettingsChanged,
    WorkspaceSelectionChanged,
    WorkspaceInventoryChanged,
    WorkspaceHeaderChanged { workspace },
    WorkspaceStatusChanged { workspace },
    WorkspaceDiffStatsChanged { workspace, additions, deletions },
    WorkspaceBranchChanged { workspace },
    WorkspaceLifecycleChanged { workspace },
    WorkspaceMetadataChanged { old_workspace, workspace, branch },
    WorkspaceRuntimeChanged { workspace },
    WorkspaceReviewChanged { workspace },
    WorkspaceGitReviewChanged { workspace },
    WorkspaceChatLifecycleChanged { workspace },
    WorkspaceChatMessagesChanged { workspace, thread_id },
    ChatMessageAppended { workspace, thread_id, message_id },
    ChatMessageUpdated { workspace, thread_id, message_id },
    ChatTimelineTailChanged { workspace, thread_id },
    ChatComposerChanged { target },
    ChatQueueChanged { target },
    ChatTabChanged { workspace, thread_id },
    ChatSessionStatusChanged { workspace, thread_id, session_id },
    RightPanelFileListChanged { workspace },
    RightPanelSelectedFileChanged { workspace, path },
    RightPanelDiffPreviewChanged { workspace, path },
    ReviewCommentsChanged { workspace },
    TodosChanged { workspace },
    TerminalBufferChanged { workspace, terminal_id },
    RuntimeProcessChanged { workspace, process_id },
    SettingsSectionChanged { scope, section },
    TerminalChanged { workspace },
}
```

| Old event | Old fanout | New event(s) | New fanout | Status |
| --- | --- | --- | --- | --- |
| `Manual` | `RefreshScope::All`: sidebar, dashboard, projects, history, workspace shell. | `RefreshHub::debug_full_refresh()` | Same fanout only when `ARCHDUCTOR_GTK_DEBUG_FULL_REFRESH` is enabled. | Removed / debug-only |
| `ProjectInventoryChanged` | Projects, sidebar, dashboard. | `ProjectInventoryChanged` | Projects, sidebar, dashboard. | Preserved |
| `SettingsChanged` | Projects, workspace shell. | `SettingsChanged`; `SettingsSectionChanged { scope, section }` | `SettingsChanged` refreshes projects and workspace mount. `SettingsSectionChanged` is currently placeholder. | Split |
| `WorkspaceSelectionChanged` | Sidebar, workspace shell. | `WorkspaceSelectionChanged` | Sidebar, workspace mount. | Mount-only |
| `WorkspaceInventoryChanged` | Sidebar, dashboard, history, workspace shell. | `WorkspaceInventoryChanged`; `WorkspaceLifecycleChanged { workspace }` | Both route inventory/lifecycle changes to sidebar, dashboard, history, and workspace mount. | Split / mount-only |
| `WorkspaceMetadataChanged { old_workspace, workspace, branch }` | Workspace nav row only. | `WorkspaceHeaderChanged`, `WorkspaceStatusChanged`, `WorkspaceDiffStatsChanged`, `WorkspaceBranchChanged`, `WorkspaceLifecycleChanged`, compatibility `WorkspaceMetadataChanged` | Header/status/branch/metadata/diff route to workspace nav row. `WorkspaceDiffStatsChanged` updates sidebar row diff labels. `WorkspaceMetadataChanged` still handles rename and optional branch text. | Split |
| `WorkspaceRuntimeChanged { workspace }` | Sidebar, dashboard, history, runtime. | `WorkspaceRuntimeChanged`; `RuntimeProcessChanged { workspace, process_id }` | `WorkspaceRuntimeChanged` routes only to runtime. `RuntimeProcessChanged` is placeholder. | Split |
| `WorkspaceReviewChanged { workspace }` | Dashboard, history, review. | `WorkspaceReviewChanged`; `ReviewCommentsChanged { workspace }` | `WorkspaceReviewChanged` routes only to review. `ReviewCommentsChanged` is placeholder. | Split |
| `WorkspaceGitReviewChanged { workspace }` | Review and workspace nav row. | `WorkspaceGitReviewChanged` | Review and workspace nav row. | Preserved |
| `WorkspaceChatLifecycleChanged { workspace }` | Sidebar, dashboard, history, chat tabs. | `WorkspaceChatLifecycleChanged`; `ChatTabChanged`; `ChatSessionStatusChanged` | `WorkspaceChatLifecycleChanged` routes only to chat tabs. `ChatTabChanged` and `ChatSessionStatusChanged` are placeholders. | Split |
| `WorkspaceChatMessagesChanged { workspace, thread_id }` | Chat surface only. | `WorkspaceChatMessagesChanged`; `ChatMessageAppended`; `ChatMessageUpdated`; `ChatTimelineTailChanged` | All route to chat surface. | Split |
| `TerminalChanged { workspace }` | Sidebar, dashboard, history, runtime. | `TerminalChanged`; `TerminalBufferChanged { workspace, terminal_id }`; `RuntimeProcessChanged { workspace, process_id }` | `TerminalChanged` routes only to runtime. `TerminalBufferChanged` and `RuntimeProcessChanged` are placeholders. | Split |

## New Refresh Events By Owner

### Workspace

| New event | Intended owner | Current route | Current handler coverage |
| --- | --- | --- | --- |
| `WorkspaceHeaderChanged { workspace }` | Header/title row and nav row. | Workspace nav row. | Placeholder in mounted sidebar handler. |
| `WorkspaceStatusChanged { workspace }` | Workspace status label and row/card status. | Workspace nav row. | Placeholder in mounted sidebar handler. |
| `WorkspaceDiffStatsChanged { workspace, additions, deletions }` | Sidebar/dashboard/history row/card diff labels. | Workspace nav row. | Implemented for sidebar row additions/deletions labels. |
| `WorkspaceBranchChanged { workspace }` | Branch label and row/card branch metadata. | Workspace nav row. | Placeholder unless paired with `WorkspaceMetadataChanged { branch: Some(..) }`. |
| `WorkspaceLifecycleChanged { workspace }` | Structural mount changes: create/delete/archive/restore. | Sidebar, dashboard, history, workspace mount. | Implemented as mount/inventory fanout. |
| `WorkspaceMetadataChanged { old_workspace, workspace, branch }` | Compatibility rename/branch update event. | Workspace nav row. | Implemented for sidebar rename and optional branch text. |

### Chat

| New event | Intended owner | Current route | Current handler coverage |
| --- | --- | --- | --- |
| `WorkspaceChatMessagesChanged { workspace, thread_id }` | Chat surface timeline. | Chat surface. | Implemented. |
| `ChatMessageAppended { workspace, thread_id, message_id }` | Chat surface row append. | Chat surface. | Event route exists; callers still mostly use `WorkspaceChatMessagesChanged`. |
| `ChatMessageUpdated { workspace, thread_id, message_id }` | Chat surface row update. | Chat surface. | Event route exists; callers still mostly use `WorkspaceChatMessagesChanged`. |
| `ChatTimelineTailChanged { workspace, thread_id }` | Chat surface tail diff. | Chat surface. | Event route exists; callers still mostly use `WorkspaceChatMessagesChanged`. |
| `ChatComposerChanged { target }` | Composer for one target. | No-op placeholder. | Not wired. |
| `ChatQueueChanged { target }` | Queue overlay for one target. | No-op placeholder. | Not wired. |
| `ChatTabChanged { workspace, thread_id }` | Tab strip button for one thread. | No-op placeholder. | Not wired. |
| `ChatSessionStatusChanged { workspace, thread_id, session_id }` | Session status indicator/badge. | No-op placeholder. | Not wired. |
| `WorkspaceChatLifecycleChanged { workspace }` | Thread-nav snapshot/tab strip. | Chat tabs. | Implemented. |

### Sidebar, Dashboard, History

| New event | Intended owner | Current route | Current handler coverage |
| --- | --- | --- | --- |
| `ProjectInventoryChanged` | Project-level list/card rebuilds. | Projects, sidebar, dashboard. | Implemented. |
| `WorkspaceInventoryChanged` | Workspace structural list/card rebuilds. | Sidebar, dashboard, history, workspace mount. | Implemented. |
| `WorkspaceMetadataChanged` | Affected workspace row. | Workspace nav row. | Implemented for sidebar row rename/branch. |
| `WorkspaceDiffStatsChanged` | Affected workspace row/card diff labels. | Workspace nav row. | Implemented for sidebar row labels. Dashboard/history card handlers are not wired in this commit. |
| `WorkspaceRuntimeChanged` | Affected runtime badge/count only. | Runtime only. | Sidebar/dashboard/history no longer refresh from this event. Targeted row/card runtime handlers are not wired in this commit. |
| `WorkspaceChatLifecycleChanged` | Affected chat badge/count only. | Chat tabs only. | Sidebar/dashboard/history no longer refresh from this event. Targeted row/card chat handlers are not wired in this commit. |
| `WorkspaceReviewChanged` | Affected review badge/count only. | Review only. | Dashboard/history no longer refresh from this event. Targeted card review handlers are not wired in this commit. |

### Right Panel

| New event | Intended owner | Current route | Current handler coverage |
| --- | --- | --- | --- |
| `RightPanelFileListChanged { workspace }` | Mounted right-panel file list. | Right-panel file-list slot. | Implemented in workspace file panel. |
| `RightPanelSelectedFileChanged { workspace, path }` | Mounted file preview/editor/diff for one file. | Right-panel diff-preview slot. | Implemented in workspace file panel. |
| `RightPanelDiffPreviewChanged { workspace, path }` | Mounted diff preview/editor/preview for one file. | Right-panel diff-preview slot. | Implemented in workspace file panel. |

### Other Panels

| New event | Intended owner | Current route | Current handler coverage |
| --- | --- | --- | --- |
| `ReviewCommentsChanged { workspace }` | Local review list/count. | No-op placeholder. | Not wired. |
| `TodosChanged { workspace }` | Todo list/count. | No-op placeholder. | Not wired. |
| `TerminalBufferChanged { workspace, terminal_id }` | One terminal buffer. | No-op placeholder. | Not wired. |
| `RuntimeProcessChanged { workspace, process_id }` | One runtime process row/status. | No-op placeholder. | Not wired. |
| `SettingsSectionChanged { scope, section }` | One settings section. | No-op placeholder. | Not wired. |

## Handler Slot Breakdown

| Old slot/API | Old purpose | New slot/API | Status |
| --- | --- | --- | --- |
| `set_sidebar` | Full sidebar callback. | `set_sidebar` | Preserved for inventory/page-level updates. |
| `set_dashboard` | Full dashboard callback. | `set_dashboard` | Preserved for project/workspace inventory updates. |
| `set_projects` | Projects/settings callback. | `set_projects` | Preserved. |
| `set_history` | Full history callback. | `set_history` | Preserved for inventory/lifecycle updates. |
| `set_workspace` | Alias for workspace shell. | Removed. | Replaced by explicit `set_workspace_mount`. |
| `set_workspace_shell` | Whole workspace shell callback. | `set_workspace_mount` | Mount-only. |
| `set_workspace_chat_surface` | Whole chat surface refresh callback. | `set_workspace_chat_surface` | Preserved, but now reached only by chat message/timeline events. |
| `set_workspace_chat_tabs` | Workspace chat tab strip callback. | `set_workspace_chat_tabs` | Preserved, no shell fallback. |
| `set_workspace_runtime` | Workspace runtime callback. | `set_workspace_runtime` | Preserved, no shell fallback. |
| `set_workspace_review` | Workspace review callback. | `set_workspace_review` | Preserved, no shell fallback. |
| `set_workspace_nav_row` | Sidebar row metadata callback. | `set_workspace_nav_row` | Expanded to receive split workspace row events. |
| none | Mounted right-panel file list. | `set_right_panel_file_list` | Added. |
| none | Mounted right-panel file diff/preview. | `set_right_panel_diff_preview` | Added. |

## Metric Breakdown

Before metrics:

- `sidebar`
- `dashboard`
- `projects`
- `history`
- `workspace_shell`
- `workspace_chat_surface`
- `workspace_chat_tabs`
- `workspace_runtime`
- `workspace_review`
- `workspace_nav_row`

After metrics:

- all old metrics except broad `RefreshScope::All`/`Workspace` entry points
- `right_panel_file_list`
- `right_panel_diff_preview`

Acceptance check:

- routine chat message events should not increment sidebar, dashboard, history,
  workspace shell, chat tabs, runtime, or review
- routine runtime/terminal events should not increment sidebar, dashboard,
  history, or workspace shell
- routine review events should not increment dashboard, history, or workspace
  shell
- right-panel file/diff events should not increment workspace shell or chat
  surface

## Migrated Call Sites

| Old caller behavior | New caller behavior | File |
| --- | --- | --- |
| Register workspace shell through `set_workspace`. | Register mount callback through `set_workspace_mount`. | `main.rs` |
| Keyboard refresh used `RefreshScope::All`. | Keyboard refresh calls `debug_full_refresh()`, gated by `ARCHDUCTOR_GTK_DEBUG_FULL_REFRESH`. | `main.rs` |
| Command-palette refresh used `RefreshScope::All`. | Command-palette refresh calls `debug_full_refresh()`, gated by `ARCHDUCTOR_GTK_DEBUG_FULL_REFRESH`. | `main.rs` |
| Palette run command refreshed whole workspace after command completion. | Emits `TerminalChanged { workspace }`. | `main.rs` |
| File save refreshed whole workspace. | Emits `RightPanelDiffPreviewChanged { workspace, path }`. | `workspace_command_center.rs` |
| Mounted file list loaded only on panel build. | File list can reload from `RightPanelFileListChanged`. | `workspace_command_center.rs` |
| Mounted file diff/preview loaded only by open/reload buttons. | Diff/preview can reload from `RightPanelSelectedFileChanged` or `RightPanelDiffPreviewChanged`. | `workspace_command_center.rs` |
| Branch action refreshed whole workspace. | Checkout/rename still emit `WorkspaceMetadataChanged`; all branch actions also emit `WorkspaceBranchChanged`. | `workspace_command_center.rs` |
| Checkpoint create refreshed whole workspace. | Emits `WorkspaceStatusChanged`. | `workspace_command_center.rs` |
| Checkpoint restore refreshed whole workspace. | Emits `WorkspaceDiffStatsChanged`. | `workspace_command_center.rs` |
| Link/unlink directory refreshed whole workspace. | Emits `RightPanelFileListChanged`. | `workspace_command_center.rs` |
| Conflict copy all refreshed whole workspace. | Emits `RightPanelFileListChanged`. | `workspace_command_center.rs` |
| Conflict copy one file refreshed whole workspace. | Emits `RightPanelFileListChanged`. | `workspace_command_center.rs` |
| Sidebar row metadata handler only handled rename. | Sidebar stores diff label refs and updates additions/deletions from `WorkspaceDiffStatsChanged`. | `sidebar.rs` |

## Current Gaps

This commit removed the broad controls and added the event surface, but not
every planned small event has a mounted child handler yet.

Known placeholder events:

- `WorkspaceHeaderChanged`
- `WorkspaceStatusChanged`
- `WorkspaceBranchChanged`
- `ChatComposerChanged`
- `ChatQueueChanged`
- `ChatTabChanged`
- `ChatSessionStatusChanged`
- `ReviewCommentsChanged`
- `TodosChanged`
- `TerminalBufferChanged`
- `RuntimeProcessChanged`
- `SettingsSectionChanged`

Known partially wired events:

- `WorkspaceDiffStatsChanged` updates sidebar row diff labels; dashboard and
  history card diff labels are not wired.
- `ChatMessageAppended`, `ChatMessageUpdated`, and `ChatTimelineTailChanged`
  route to the chat surface, but most current callers still emit
  `WorkspaceChatMessagesChanged`.
- `RightPanelFileListChanged` and `RightPanelDiffPreviewChanged` are wired for
  the workspace file panel. They do not currently update other right-panel
  sections.

## Tests Guarding The Change

`crates/gtk-app/src/refresh.rs` now guards the refactor with tests that assert:

- routine source files do not call `RefreshScope::All`
- routine source files do not call `RefreshScope::Workspace`
- routine source files do not use the obsolete `set_workspace` alias
- `run_event_or_shell` is absent from production `refresh.rs`
- `WorkspaceRefreshTarget::Shell` is absent from production `refresh.rs`
- unregistered granular handlers do not fall back to workspace shell
- chat messages update only chat surface
- chat lifecycle updates only chat tabs
- runtime updates only runtime
- review updates only review
- git review updates review plus nav row
- right-panel diff preview updates only the right-panel child slot

Broader GTK tests also cover sidebar row behavior, chat refresh paths, terminal
refresh event routing, and right-panel tab persistence.
