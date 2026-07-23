# Codex And Claude Event Mapping

This maps how Codex app-server and Claude stream-json events become Archductor
provider events, then how those provider events render as read, edit, and ran
actions.

## Canonical Archductor Target

Archductor persists provider output as `ProviderEventRecord`s. The canonical
event kind is `ProviderEventKind` in
`crates/core/src/provider_events.rs`.

For read/edit/ran, the projection target is:

| User-facing action | Canonical provider kind | Subtype expectation | Projection result |
| --- | --- | --- | --- |
| Ran command | `CommandProcess` | any command subtype | `Command` |
| Read file | `FileSystem` | no `write`, `create`, `patch`, or `edit` token | `FileRead` |
| Edited file | `FileSystem` | contains `edit` or `patch` | `FilePatch` |
| Turn-level file diff | `DiffFileChange` | any diff/change subtype | `FileDiff` |

The projection rule lives in `provider_projection_category`:

- `CommandProcess` always becomes `Command`.
- `FileSystem` plus `write`/`create` becomes `FileWrite`.
- `FileSystem` plus `patch`/`edit` becomes `FilePatch`.
- Other `FileSystem` becomes `FileRead`.
- `DiffFileChange` becomes `FileDiff`.
- Generic `Tool` becomes `NativeTool`.

That means the provider adapters must classify native tool calls correctly
before persistence. The projection layer already has enough information if the
adapter gives it the right `kind` and `provider_subtype`.

## Codex App-Server Flow

Codex native JSONL enters through
`crates/core/src/provider_adapters/codex_app_server.rs`.

Flow:

1. `read_jsonl_messages` reads app-server JSONL.
2. `parse_jsonl_message` detects notification/request/response shape.
3. `CodexAppServerMessage::to_provider_event_draft` derives a Codex event name.
4. `codex_event_name` rewrites lifecycle items like `item/completed` to
   `item/{item.type}/completed` when `params.item.type` exists.
5. `classify_codex_method` tokenizes the derived name and assigns a Codex
   category.
6. `codex_category_to_provider_kind` maps that category to Archductor
   `ProviderEventKind`.
7. `CodexProviderEventDraft::into_provider_event_draft` persists the canonical
   fields, with `provider_subtype` set to the derived Codex name.

Current Codex mappings:

| Native Codex shape | Current derived name | Current Archductor kind | Current projection |
| --- | --- | --- | --- |
| `item.type = commandExecution` | `item/commandExecution/{phase}` | `CommandProcess` | `Command` |
| `item.type = fileChange` | `item/fileChange/{phase}` | `DiffFileChange` | `FileDiff` |
| method/name contains `file`, `path`, `directory` | original method/name | `FileSystem` | usually `FileRead` |
| method/name contains `patch`, `edit`, `change` | original method/name | `DiffFileChange` | `FileDiff` |
| `item.type = dynamicToolCall` | `item/dynamicToolCall/{phase}` | `Tool` | `NativeTool` |
| `item.type = mcpToolCall` | `item/mcpToolCall/{phase}` | `Mcp` or `Tool` depending method tokens | `McpTool` or `NativeTool` |

Codex status:

- `commandExecution` already maps to ran-command semantics.
- `fileChange` already maps to turn-level file diff semantics.
- Dynamic/native tool calls named `Read`, `Edit`, or `Bash` currently stay
  generic because classification only sees `dynamicToolCall`, not the tool
  name or arguments.
- If Codex emits read/edit/ran as `dynamicToolCall`, Archductor will render them
  as generic tool cards instead of canonical read/edit/ran action cards.

## Claude Stream-JSON Flow

Claude native JSONL enters through
`crates/core/src/provider_adapters/claude_stream.rs`.

Flow:

1. `parse_claude_stream_json_lines` feeds each JSON object into
   `ClaudeStreamParser`.
2. `kind_for` maps stream-json record shapes to `ClaudeProviderEventKind`.
3. `apply_identity_state` tracks message IDs, tool-use IDs, block indexes, and
   tool names across `content_block_start`, `content_block_delta`, and
   `content_block_stop`.
4. `ClaudeProviderEventDraft::into_provider_event_draft` maps
   `ClaudeProviderEventKind` to Archductor `ProviderEventKind`.
5. The normalized payload preserves `tool_name`, body text, usage, cost, and
   duration.

Current Claude mappings:

| Native Claude shape | Current Claude kind | Current Archductor kind | Current projection |
| --- | --- | --- | --- |
| `content_block_start` with `tool_use` | `ToolUse` | `Tool` | `NativeTool` |
| `content_block_delta` with `input_json_delta` | `ToolInputDelta` | `Tool` | `NativeTool` |
| `content_block_stop` for a tracked tool block | `ToolResult` | `Tool` | `NativeTool` |
| top-level/user `tool_result` | `ToolResult` or `DeferredResult` | `Tool` | `NativeTool` |
| `Bash` tool name | preserved in payload | still `Tool` | `NativeTool` |
| `Read` tool name | preserved in payload | still `Tool` | `NativeTool` |
| `Edit` tool name | preserved in payload | still `Tool` | `NativeTool` |

Claude status:

- Tool identity tracking is good: block index, tool-use ID, and tool name are
  preserved.
- Canonical classification is too coarse: `claude_kind_to_provider_kind` only
  sees `ClaudeProviderEventKind`, so all tool names become generic `Tool`.
- Because the canonical kind is generic, `Bash`, `Read`, and `Edit` do not reach
  the projection as ran/read/edit.

## Correctness Target

The adapter layer should promote known local tool names before persistence:

| Provider | Native tool name or item | Desired kind | Desired subtype |
| --- | --- | --- | --- |
| Claude | `Bash` | `CommandProcess` | `command` |
| Claude | `Read` | `FileSystem` | `read` |
| Claude | `Edit`, `MultiEdit`, `NotebookEdit` | `FileSystem` | `edit` |
| Claude | `Write` | `FileSystem` | `write` |
| Codex | `commandExecution` | `CommandProcess` | existing Codex name is acceptable |
| Codex | `fileChange` | `DiffFileChange` | existing Codex name is acceptable |
| Codex | `dynamicToolCall` with `tool = Bash` | `CommandProcess` | `command` |
| Codex | `dynamicToolCall` with `tool = Read` | `FileSystem` | `read` |
| Codex | `dynamicToolCall` with `tool = Edit`/`MultiEdit` | `FileSystem` | `edit` |
| Codex | `dynamicToolCall` with `tool = Write` | `FileSystem` | `write` |

Generic or unknown provider tools should remain `ProviderEventKind::Tool`.
MCP tools should remain `ProviderEventKind::Mcp`.

## Where To Fix Later

Do not fix this in GTK. GTK should consume the shared projection.

Best fix points:

- Codex: in `CodexProviderEventDraft::into_provider_event_draft`, derive the
  canonical kind/subtype from `params.item.type`, `params.item.tool`, and
  `params.item.arguments` before building `ProviderEventDraft`.
- Claude: in `ClaudeProviderEventDraft::into_provider_event_draft`, choose the
  canonical kind/subtype using `self.kind`, `self.tool_name`, and raw tool input.
  `claude_kind_to_provider_kind` can remain the fallback for non-tool events.
- Projection: leave `provider_projection_category` alone unless the product
  language changes. It already maps command/file-system/file-diff kinds into
  the right UI categories.

Regression tests should cover canonical conversion for each provider:

- Claude `Bash` becomes `CommandProcess`/`command`.
- Claude `Read` becomes `FileSystem`/`read`.
- Claude `Edit` becomes `FileSystem`/`edit`.
- Codex `dynamicToolCall` `Bash` becomes `CommandProcess`/`command`.
- Codex `dynamicToolCall` `Read` becomes `FileSystem`/`read`.
- Codex `dynamicToolCall` `Edit` becomes `FileSystem`/`edit`.
- Existing generic unknown tools remain `Tool`.
- Existing MCP tool calls remain `Mcp`.

## Bottom Line

The shared projection is ready for read/edit/ran. The current mismatch is in
provider adapter classification:

- Codex structured `commandExecution` and `fileChange` are mapped.
- Codex dynamic tool calls are not mapped by native tool name.
- Claude preserves native tool names but still maps every tool event to generic
  `Tool`.

This is the branch-sized work if we dedicate a branch to provider event mapping.
