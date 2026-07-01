# Settings Redesign Design

**Goal:** Redesign the GTK Settings surface so it feels like a dense, panel-like Archductor inspector instead of a generic form page, while keeping the current settings data model and load/save behavior intact.

## Scope

This pass addresses:

- smoother settings surface with better formatted text fields
- align the Settings UI with the Archductor-style dark shell language

This pass does **not**:

- change the repository settings schema
- change load/save semantics
- redesign the entire app shell
- change workspace/project flows outside what Settings visually depends on

## Current Problems

- The Settings page still reads like a normal web form page instead of an app-native inspector surface.
- Inputs are grouped in wide horizontal rows that make labels, fields, and helper context hard to scan.
- Large text editors feel dropped in rather than intentionally framed.
- The current theme uses rounder, higher-contrast card styling than the target
  Archductor design language.

## UX Direction

Settings should feel like a panel-style editor inside the existing shell:

- top control bar for repository selection, settings layer, and load/save actions
- narrow left section rail for `General`, `Prompts`, `Providers`, `Git`, and `Advanced`
- main content pane on the right with grouped setting blocks
- machine-owned inputs and editors use monospace; human-facing labels and helper copy use sans

## Layout

### Outer Surface

- Keep the existing Settings route/page for now
- Replace the tab-strip feel with a split inspector surface
- Left rail stays compact and persistent while the right pane changes section content

### Section Rail

Each section row should show:

- title
- one-line purpose copy
- solid active state

This should read like a native list/inspector navigation column, not browser tabs.

### Content Pane

The right pane should contain titled groups such as:

- `Scripts`
- `Runtime flags`
- `Environment`
- `Provider paths`
- `Git behavior`
- `Files to copy`
- `Prompt editors`
- `Advanced customization`

Within each group:

- labels are readable and short
- helper copy explains the field in one line where useful
- fields stack vertically or use restrained two-column arrangements when that improves scanability

### Editor Blocks

These should become dedicated editor surfaces:

- environment variables
- file-copy globs
- prompt bodies
- advanced customization TOML

They should:

- use monospace
- have stronger visual framing than plain entries
- use heights that feel intentional instead of arbitrary

## Visual Language

Follow the Archductor-style dark shell guidance:

- darker shell surfaces with separation by lightness, not heavy borders/shadows
- one green semantic accent
- three text levels only: primary, secondary, muted
- tighter spacing and denser rows
- no decorative card treatment

## Implementation

### Files

- Modify `crates/gtk-app/src/settings.rs`
  - replace the current tab-strip structure with a split inspector layout
  - introduce compact section metadata/helpers so section order and copy are explicit
  - create reusable group/field/editor helpers for consistent formatting

- Modify `crates/gtk-app/src/theme.rs`
  - add settings-specific inspector styles
  - adjust settings surface colors, spacing, field framing, and rail states to match the approved direction

## Testing

- Add small unit coverage for any new section metadata/helper logic introduced in `settings.rs`
- Run `cargo test -p linux-archductor-gtk -- --nocapture`

## Risks

- GTK layout changes can regress spacing or overflow if rows are too rigid
- Theme updates must stay scoped enough not to accidentally restyle unrelated surfaces

## Success Criteria

- Settings no longer feels like a generic page with loose rows
- Section navigation reads like an inspector rail
- Text fields and text editors are easier to scan and edit
- The settings surface is visibly closer to the Archductor design language
