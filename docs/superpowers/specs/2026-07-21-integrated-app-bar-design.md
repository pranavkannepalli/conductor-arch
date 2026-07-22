# Integrated App Bar Design

## Goal

Replace Archductor's empty draggable title bar and duplicated in-page headers
with one persistent app bar. The bar must keep navigation, page identity, and
workspace review context at one consistent height so future layout changes do
not need to reconcile several independent header implementations.

## Chosen Approach

Build one application-owned app-bar container in `main.rs`, outside the page
stack. Its content changes when `AppState` navigation changes, while its outer
container, height, styling, and window integration remain stable.

This avoids moving live GTK widgets between parents and avoids maintaining a
different title bar for each page. Page builders will return body content only;
they will no longer construct their existing top-level page headers.

## Layout

The app bar has three stable regions whose widths track the three visible app
columns:

1. A sidebar region containing Back and Forward plus sidebar controls. Its
   width and collapsed state track the application sidebar.
2. A workspace-center region containing page/workspace identity, repository,
   and branch context. Its width tracks the start pane of the workspace split.
3. A review region containing PR/status and trailing actions. Its width and
   visibility track the workspace review pane, followed by GTK title buttons.

The region separators align exactly with the sidebar and workspace pane
dividers below them. The bar therefore reads as the natural upward extension
of the application's columns, not as an independent toolbar floating above
them. On non-workspace pages, the page header spans the center and review
regions while the sidebar region remains aligned.

For a workspace, the context region shows the workspace name and the existing
repository/branch identity. The trailing region contains the controls currently
owned by the workspace header, including editor/sidebar actions and the compact
PR or workspace status presentation that is relevant to the selected
workspace.

For Dashboard, Projects, Settings, and History, the context region contains
the existing page title and supporting header content. Existing subtitles,
tabs, filters, and header actions remain available, but are composed into the
shared bar rather than duplicated in the page body.

The bar preserves the current header visual language: background, border,
typography, button treatment, and spacing. Its outer height is shared across
all page states. Long workspace, repository, branch, and page text ellipsizes
instead of increasing the bar height.

## Navigation and State

The app bar observes the same `AppState` that controls the main stack. A small
page-context projection describes the visible page, labels, selected workspace,
PR/status state, and available actions. Navigation or workspace refresh events
update that projection without rebuilding the entire page.

Back and Forward remain global controls. They are always placed in the same
location and become insensitive when the corresponding history direction is
unavailable.

Workspace renames, branch changes, PR refreshes, and status changes refresh the
app-bar projection through the existing typed refresh/state mechanisms. The
bar must not introduce its own independent source of workspace truth.

## Platform Behavior

The shared app bar is the sole title bar on Linux and Windows. It provides GTK
title buttons and draggable window movement on both platforms. Archductor must
not render a second native or GTK caption above it, and no separate top bar may
show only the text “Archductor.”

The window title remains available to the operating system for task switchers
and accessibility metadata, but it is not rendered as an additional caption
inside the window.

## Page Changes

- Dashboard removes its in-page dashboard header and contributes its title,
  subtitle, and project filter context across the center/review bar regions.
- Projects removes its in-page page header and contributes its title and
  subtitle.
- Settings removes its in-page page header and contributes its title,
  subtitle/scope context, and any header-level controls.
- History removes its in-page page header and contributes its title, subtitle,
  and Workspaces/Chats tabs.
- Workspace removes the repository/branch header row from the center panel and
  contributes workspace identity and repository/branch context to the center
  region, with PR/status and existing header actions in the review region.
- Workspace creation and failure states use the same workspace app-bar context;
  their body retains only progress, error, and recovery controls.

## Error and Empty States

If no workspace is selected, workspace-specific app-bar content falls back to
the active non-workspace page context rather than showing stale workspace data.
If workspace or PR data cannot be loaded, the bar keeps the workspace identity
and shows a compact unavailable/error state; detailed failure text remains in
the page body or toast. Missing data must never change the bar height.

## Testing and Verification

Written GTK tests will cover:

- the page-to-app-bar context projection;
- Back and Forward sensitivity;
- workspace identity, branch, status, and PR variants;
- equal-height/single-row styling contracts and ellipsizing;
- removal of legacy in-page header construction; and
- aligned sidebar/center/review region widths and collapsed states; and
- one custom titlebar on both Linux and Windows, with no duplicate caption.

Verification will run focused GTK tests first, then the complete GTK package
tests and build. A GTK runtime smoke will navigate Dashboard, Projects,
Settings, History, a workspace without a PR, and a workspace with PR state to
confirm that the bar stays fixed-height and draggable where supported. A
relevant CLI smoke will confirm that shared workspace state still reaches the
fallback command boundary; no CLI presentation change is required because the
feature is window chrome.

## Scope Boundaries

This change does not redesign the sidebar, page bodies, PR workflows, or
navigation history semantics. It does not add new workspace or PR actions. It
only relocates existing header information and controls into a persistent,
equal-height app bar and removes the replaced headers.
