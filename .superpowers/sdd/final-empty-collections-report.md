# Final Review: Explicit Empty Collection Precedence

## Result

Core Settings now distinguishes an absent collection from an explicitly present
empty `[]` or `{}` while merging built-in, app Shared, repository-committed, and
Local layers. Absent collections inherit. Explicit empty collections clear the
inherited value.

Commit: `fix(settings): preserve empty collection overrides`

## Root Cause

Task 2 kept settings raw until the final effective merge, which fixed presence
for string-encoded Files to Copy values. The remaining collection fields still
discarded presence inside their raw structs:

- `#[serde(default)] Vec<_>` made absent and `[]` identical.
- A bare `BTreeMap` made absent and `{}` identical for view colors.
- Environment-variable and agent-profile maps were already optional, but merge
  eagerly converted both absent and empty maps to `default()`.
- List merges then used `is_empty() => inherit`, so an explicit clear inherited.

The same raw merge serves every layer boundary, so the defect affected Shared,
repository-committed, and Local settings.

## Field Inventory

| Typed field | Raw encoding before | Fix |
| --- | --- | --- |
| `file_include_globs` | `Option<String>` | Already presence-aware; unchanged |
| `env_file_refs` | `Option<String>` | Already presence-aware; unchanged |
| `environment_variables` | `Option<BTreeMap<...>>` | Preserve `Some(empty)` during map merge |
| `customization.agent_profiles` | `Option<BTreeMap<...>>` | Preserve `Some(empty)` during profile-map merge |
| `customization.naming.pr_body_sections` | `Vec<String>` | `Option<Vec<String>>` |
| `customization.automation.required_local_files` | `Vec<String>` | `Option<Vec<String>>` |
| `customization.agent_profiles.*.mcp_servers` | `Vec<String>` | `Option<Vec<String>>` |
| `customization.view.colors` | `BTreeMap<String, String>` | `Option<BTreeMap<String, String>>` |
| `customization.view.dashboard_columns` | `Vec<String>` | `Option<Vec<String>>` |
| `customization.view.notification_rules` | `Vec<String>` | `Option<Vec<String>>` |
| `customization.view.command_palette_presets` | `Vec<String>` | `Option<Vec<String>>` |

Non-empty maps retain the existing key-overlay behavior. Explicitly empty maps
replace the inherited map. Scalar `Option<String>` fields still preserve
explicit empty strings.

## TDD Evidence

### RED

Production code was unchanged when these tests were first run:

- `cargo test -p archductor-core raw_shared_empty_collections_clear_builtin_collections -- --nocapture`
  - Exit 101 at `environment_variables`; the Shared `{}` inherited the built-in map.
- `cargo test -p archductor-core effective_settings_repository_empty_collections_clear_shared_collections -- --nocapture`
  - Exit 101 at `environment_variables`; repository `{}` inherited Shared values.
- `cargo test -p archductor-core effective_settings_local_empty_collections_clear_repository_collections -- --nocapture`
  - Exit 101 at `environment_variables`; Local `{}` inherited repository values.

Each test continues through vector, profile-map, MCP-server, color-map,
notification, dashboard, command-preset, PR-section, and local-file assertions
after the first failing map assertion is fixed.

### GREEN

- `cargo test -p archductor-core empty_collections -- --nocapture`
  - 3 passed.
- `cargo test -p archductor-core settings::tests -- --nocapture`
  - 38 passed.
  - Includes absent-collection inheritance and explicit-empty-string compatibility.
- `cargo test -p archductor-core --all-targets`
  - 484 unit tests, 2 PTY fixture tests, and 8 session-event integration tests passed.
- `cargo check -p archductor-core --all-targets`
  - Exit 0.
- `cargo clippy -p archductor-core --all-targets -- -D warnings`
  - Exit 0, no warnings.
- `cargo fmt --all -- --check`
  - Exit 0.
- `cargo test -p archductor --test cli_sessions cli_session_open_applies_app_shared_launch_settings -- --nocapture`
  - 1 passed; app Shared settings reached the CLI launch boundary.
- `cargo test -p archductor-gtk gtk_view_preferences_use_app_shared_settings -- --nocapture`
  - 1 passed; app Shared settings reached the GTK settings-consumer boundary.

No visual GTK launch was required because this changes non-visible core merge
semantics; the focused GTK app-aware boundary test compiled and exercised the
consumer path.

## Compatibility And Risks

- Public typed settings structs and TOML field names are unchanged.
- Existing non-empty serialization and map-overlay semantics are unchanged.
- Typed serializers continue omitting default empty collections, preserving
  existing import/export output. Explicit empty syntax is honored when it is
  present in a raw layer file.
- Presence remains an internal raw-layer concern. Once projected into the
  public typed settings value, absent and empty collections are intentionally
  both represented by the existing empty collection type.
- No database, CLI command, or GTK UI code changed.
