# Theme And Icons Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve the GTK app's dark theme, typography, contrast, and icon reliability.

**Architecture:** Keep the existing GTK layout and widget classes. Add a central icon alias resolver in `buttons.rs`, update direct icon image call sites to use it, and append a coherent dark-slate theme override layer in `theme.rs` so the visual refresh is scoped and easy to iterate.

**Tech Stack:** Rust, GTK4/libadwaita CSS, existing GTK widget classes.

---

### Task 1: Icon Alias Resolver

**Files:**
- Modify: `crates/gtk-app/src/buttons.rs`
- Modify: `crates/gtk-app/src/session_surface.rs`

- [ ] Add tests for icon aliases: `send-symbolic`, `focus-windows-symbolic`, `code-symbolic`, `zed-symbolic`, and `sidebar-hide-symbolic`.
- [ ] Implement `resolve_icon_name`.
- [ ] Use `resolve_icon_name` in `icon_button` and direct `Image::from_icon_name` calls in `session_surface.rs`.
- [ ] Run `cargo test -p linux-archductor-gtk buttons session_surface::tests::editor_choices_use_resolvable_icons -- --nocapture`.

### Task 2: Dark Slate Theme Refresh

**Files:**
- Modify: `crates/gtk-app/src/theme.rs`

- [ ] Add a focused CSS test that asserts the refreshed theme includes the new surface, text, accent, and focus colors.
- [ ] Append a theme override section with dark-slate surfaces, modern font stacks, clearer hover/focus states, and upgraded chat/tool/context styling.
- [ ] Run `cargo test -p linux-archductor-gtk theme -- --nocapture`.

### Task 3: Verification

**Files:**
- No new files beyond this plan.

- [ ] Run `cargo fmt --all`.
- [ ] Run `cargo test -p linux-archductor-gtk buttons theme session_surface -- --nocapture` using valid individual filters if Cargo rejects multiple filters.
- [ ] Run `cargo check --workspace`.
- [ ] Run `git diff --check`.
