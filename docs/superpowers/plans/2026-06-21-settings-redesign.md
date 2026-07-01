# Settings Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the GTK Settings page into a panel-like Archductor-style inspector surface without changing settings persistence behavior.

**Architecture:** Keep the Settings route, but replace its tab-strip layout with a split editor surface. Put section metadata and small formatting helpers in `settings.rs`, then scope the visual treatment in `theme.rs` so the redesign is mostly local to Settings.

**Tech Stack:** Rust, GTK4/libadwaita, existing `linux-archductor-core` settings APIs

---

### Task 1: Add Small Settings Surface Metadata

**Files:**
- Modify: `crates/gtk-app/src/settings.rs`
- Test: `crates/gtk-app/src/settings.rs`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn settings_sections_keep_expected_order() {
    let sections = settings_sections();
    let ids = sections.iter().map(|section| section.id).collect::<Vec<_>>();
    assert_eq!(ids, vec!["general", "prompts", "providers", "git", "advanced"]);
}

#[test]
fn prompt_section_uses_editor_style_fields() {
    let prompts = settings_sections()
        .into_iter()
        .find(|section| section.id == "prompts")
        .unwrap();
    assert!(prompts.description.contains("prompt"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p linux-archductor-gtk settings_sections_keep_expected_order -- --nocapture`
Expected: FAIL because `settings_sections` does not exist yet.

- [ ] **Step 3: Write minimal implementation**

Add a small `SettingsSection` helper plus `settings_sections()` in `crates/gtk-app/src/settings.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p linux-archductor-gtk settings_sections_keep_expected_order -- --nocapture`
Expected: PASS

### Task 2: Rebuild Layout As Inspector Surface

**Files:**
- Modify: `crates/gtk-app/src/settings.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn settings_sections_keep_expected_order() {
    let sections = settings_sections();
    assert_eq!(sections.first().unwrap().id, "general");
}
```

- [ ] **Step 2: Run test to verify it fails if metadata is missing**

Run: `cargo test -p linux-archductor-gtk settings_sections_keep_expected_order -- --nocapture`
Expected: PASS only after Task 1; use it as the guard while restructuring.

- [ ] **Step 3: Write minimal implementation**

Replace:
- horizontal top tab strip

With:
- top controls row
- left settings rail
- right content stack
- grouped content blocks and editor surfaces

- [ ] **Step 4: Run test to verify it still passes**

Run: `cargo test -p linux-archductor-gtk settings_sections_keep_expected_order -- --nocapture`
Expected: PASS

### Task 3: Apply Archductor-Style Settings Theme

**Files:**
- Modify: `crates/gtk-app/src/theme.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn prompt_section_uses_editor_style_fields() {
    let prompts = settings_sections()
        .into_iter()
        .find(|section| section.id == "prompts")
        .unwrap();
    assert!(prompts.description.contains("prompt"));
}
```

- [ ] **Step 2: Run test to verify it passes before styling**

Run: `cargo test -p linux-archductor-gtk prompt_section_uses_editor_style_fields -- --nocapture`
Expected: PASS; use this as a guard while styling changes stay behavior-preserving.

- [ ] **Step 3: Write minimal implementation**

Add scoped CSS classes for:
- settings shell
- section rail rows
- form groups
- field rows
- editor surfaces
- status strip

- [ ] **Step 4: Run test to verify it still passes**

Run: `cargo test -p linux-archductor-gtk prompt_section_uses_editor_style_fields -- --nocapture`
Expected: PASS

### Task 4: Verify Full GTK Surface

**Files:**
- Modify: `crates/gtk-app/src/settings.rs`
- Modify: `crates/gtk-app/src/theme.rs`

- [ ] **Step 1: Run the focused GTK settings tests**

Run: `cargo test -p linux-archductor-gtk settings_sections_keep_expected_order prompt_section_uses_editor_style_fields -- --nocapture`
Expected: PASS

- [ ] **Step 2: Run the full GTK test suite**

Run: `cargo test -p linux-archductor-gtk -- --nocapture`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-06-21-settings-redesign-design.md docs/superpowers/plans/2026-06-21-settings-redesign.md crates/gtk-app/src/settings.rs crates/gtk-app/src/theme.rs
git commit -m "feat(settings): redesign inspector surface"
```
