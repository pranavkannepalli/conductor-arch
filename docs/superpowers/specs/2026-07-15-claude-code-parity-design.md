# Claude Code Archcar Parity Design

## Goal

Make Claude Code a fully functional, provider-native Archductor agent using the
user's existing local Claude login. Claude must have the same product shape as
Codex at the Archcar boundary: thread-scoped sessions, durable turn delivery,
streamed and persisted events, queueing, interruption, model and effort
controls, permissions, questions, recovery, CLI coverage, and GTK coverage.

The fastest path is to finish the existing headless Claude transport instead of
adding another SDK runtime or returning to terminal emulation.

## Customer Outcome

A user can select Claude in the Archductor chat surface and use it as a real
coding agent:

- the first prompt starts the selected Claude thread and is not lost;
- multiple Claude chats remain attached to their own native conversations;
- assistant output and tool activity stream into the common transcript;
- ordinary follow-ups queue predictably and immediate delivery remains
  intentional;
- interrupt, model, effort, and permission-mode controls work without losing
  conversation context;
- Claude can request tool approval, ask the user questions, and present a plan;
- Archcar and GTK restarts recover the conversation and pending interaction;
- CLI and GTK expose the same behavior.

## Constraints

- Use the installed `claude` executable and its existing local Claude login.
- Do not require `ANTHROPIC_API_KEY`, Agent SDK packages, or a Node runtime.
- Do not use `--bare`, because it disables OAuth/keychain reads and skips the
  project and user context that desktop users expect.
- Do not scrape Claude's interactive terminal interface.
- Preserve user and project Claude configuration. Archductor may add
  session-scoped settings, but must not rewrite user settings files.
- Keep provider-specific mechanics behind provider-neutral Archcar requests,
  events, and persisted records.
- Every managed chat harness must implement the required Archcar baseline
  contract. Provider-specific features remain explicit optional capabilities.
- Keep Codex and Claude adapters isolated: neither adapter may import native
  protocol types, codecs, or runtime state from the other.
- Keep CLI and GTK behavior in line.
- Build on the provider-neutral queue and immediate-delivery work already in
  progress. Do not overwrite concurrent Codex changes.

## Current State

Claude is not an empty stub, but it is not usable as a product path.

The repository already launches Claude as a piped provider process with:

```text
claude -p --output-format stream-json --verbose \
  --include-partial-messages --input-format stream-json
```

`ClaudeStreamParser` also maps a useful subset of native records into canonical
provider events. A direct authenticated smoke of this framing succeeds with the
installed Claude Code CLI.

The missing behavior is above and around that transport:

- the GTK first-send and ready-queue path is hardcoded to Codex;
- generic Claude spawning is workspace-scoped rather than selected-thread
  scoped;
- Claude is marked ready before `system/init`;
- the Claude loop does not emit the same turn-completion lifecycle as Codex;
- `InterruptTurn` is a no-op;
- changing the stored model does not affect the running Claude process;
- permissions and questions are parsed optimistically but cannot be answered;
- parser coverage misses real records such as `rate_limit_event` and several
  system subtypes;
- existing tests use synthetic records and do not exercise a real Archcar to
  Claude conversation;
- old Claude branches remain in the generic PTY loop even though managed Claude
  sessions should never use them.

The declarative Claude capability list therefore describes intended coverage,
not current runtime behavior.

## Chosen Architecture

Use a persistent, headless Claude Code child process with bidirectional
`stream-json`, supervised by Archcar. Add a small local hook bridge so Claude's
documented permission and interactive-tool hooks can round-trip through
Archcar and GTK.

Introduce a narrow managed-harness contract beside the existing launch-focused
`HarnessController`. The contract standardizes the behavior Archcar consumes;
it does not standardize provider-native transports. Codex keeps its app-server
adapter and Claude keeps its stream-json adapter.

This follows Anthropic's recommended persistent streaming model for interactive
applications:

- https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode
- https://code.claude.com/docs/en/headless
- https://code.claude.com/docs/en/hooks

No new provider service is introduced. Archcar remains the single session
authority.

## Claude Launch Contract

The normal launch is:

```text
claude -p
  --input-format stream-json
  --output-format stream-json
  --verbose
  --include-partial-messages
  --replay-user-messages
  [--resume NATIVE_SESSION_ID]
  [--model MODEL]
  [--effort LEVEL]
  [--permission-mode MODE]
  [--append-system-prompt ARCHDUCTOR_ADDITION]
  [--settings SESSION_SETTINGS_JSON]
```

`--replay-user-messages` provides a native acknowledgement that an input crossed
the stdin boundary. Archcar uses it to distinguish queued-but-unsent input from
input already accepted by Claude when a process exits or Archcar restarts.

The session settings JSON merges Archductor hook entries into the effective
settings for that invocation. Omitted settings continue to come from the
user's normal user, project, and local settings. Archductor does not use
`--strict-mcp-config`, `--safe-mode`, or `--bare` during normal operation.

## Provider-Neutral Archcar Shape

Codex and Claude continue to use one logical contract:

- ensure a session for a workspace and chat thread;
- send user, review, or control input with an explicit delivery intent;
- interrupt the active turn;
- change provider controls for subsequent work;
- kill or recover the managed process;
- subscribe to readiness, turn completion, messages, errors, and interaction
  requests;
- resolve a pending provider interaction.

Provider-specific code translates this contract into Codex app-server JSON-RPC
or Claude stream-json, signals, resume launches, and hook results.

Archcar must not require GTK or the CLI to understand native Claude message
shapes. Native records are still stored losslessly for diagnostics.

## Managed Harness Contract

The existing `HarnessController` remains responsible for selecting a provider
and building a launch. `ManagedHarness` extends that existing boundary with a
descriptor and adapter factory. A new `ManagedHarnessAdapter` defines the pure
runtime boundary used after launch. Process ownership, durable queueing,
persistence, and event publication remain in the shared Archcar supervisor.

Codex and Claude implement `ManagedHarness`; Shell remains a terminal
`HarnessController`. Changing a required `ManagedHarness` or
`ManagedHarnessAdapter` method therefore produces a compile-time obligation for
both managed providers.

The contract lives in `crates/core/src/archcar/harness_contract.rs`. It exposes
provider-neutral types only:

```rust
pub const MANAGED_HARNESS_CONTRACT_VERSION: u16 = 1;

pub trait ManagedHarness: HarnessController {
    fn descriptor(&self) -> &'static HarnessDescriptor;
    fn create_adapter(&self, context: HarnessAdapterContext)
        -> Result<Box<dyn ManagedHarnessAdapter>>;
}

pub trait ManagedHarnessAdapter: Send {
    fn encode_input(&mut self, input: HarnessInput) -> Result<NativeWrite>;
    fn observe_native(&mut self, record: NativeRecord) -> Result<Vec<HarnessEffect>>;
    fn plan_control(&mut self, control: HarnessControl) -> HarnessControlPlan;
    fn recovery_plan(&self, cause: HarnessRecoveryCause) -> HarnessRecoveryPlan;
}
```

`HarnessDescriptor` includes the contract version, `SessionKind`, stable
provider key, display name, executable and preflight metadata, the complete
required feature set, and optional capability support. Registration rejects a
managed harness with an old contract version, a missing required feature, or
`Unsupported` support for a required feature.

`NativeRecord` and `NativeWrite` are opaque byte or JSON-line envelopes tagged
with the provider key. They do not expose Codex or Claude structs. Each adapter
owns its native parser, request IDs, native session IDs, turn tracker, control
translation, and resume arguments.

`HarnessEffect` is the only way an adapter requests changes to shared runtime
state. The supervisor itself records a successful process write before
accepting an acknowledgement effect. Together, the contract and supervisor
cover:

- initialized and ready;
- input accepted for writing and provider-acknowledged;
- turn started, completed, failed, interrupted, or deferred;
- canonical provider event plus lossless native payload;
- provider interaction requested or resolved;
- native session metadata and observed capability changes;
- retry, rate-limit, warning, and fatal error;
- child recycle or resume required.

`HarnessControlPlan` makes provider mechanics explicit:

- `NativeWrite` for a live protocol command;
- `Signal` for a process or process-group action;
- `RestartRequired` with the controls and resume identity to preserve;
- `Emulated` for behavior owned safely by Archcar;
- `Unsupported` with a stable reason.

The shared supervisor consumes these plans. It does not branch on Codex or
Claude wire formats.

### Required Baseline

A provider cannot register as an `ArchcarManaged` chat harness unless its
descriptor declares and its conformance tests prove all baseline behavior:

1. installation, version, and authentication preflight;
2. workspace- and chat-thread-scoped session identity;
3. launch, initialization, readiness, kill, and clean process ownership;
4. normal and immediate input delivery with durable local sequencing;
5. provider acknowledgement or an explicit safe acknowledgement fallback;
6. streamed canonical messages and lossless native diagnostic records;
7. exactly-once terminal turn lifecycle;
8. normal queueing while busy;
9. interrupt with a deterministic recovery fallback;
10. restart and resume without silently changing conversations;
11. crash recovery without duplicating acknowledged input;
12. model, effort or thinking, and permission-mode controls;
13. permission, user-question, and plan-approval interaction round trips;
14. structured retry, rate-limit, unsupported, and fatal errors;
15. capability discovery consumed consistently by CLI and GTK.

The mechanism may differ. For example, Codex can send a native interrupt while
Claude may signal and restart with `--resume`. Both satisfy the same observable
contract.

### Optional Provider Extensions

Capabilities outside the baseline are explicit extensions. Initial examples
include Codex goals, provider-native slash commands, and any future feature
that has no useful equivalent in every managed harness.

Each capability reports one support mode:

- `Native`;
- `RestartRequired`;
- `Emulated`;
- `Unsupported { reason }`.

Optional capabilities never receive fake parity. Codex may report its goal
extension as native while Claude reports it unsupported. CLI and GTK render an
optional action only when the selected harness descriptor supports it.

### Isolation Rules

- `codex_app_server.rs` contains Codex JSON-RPC parsing and encoding only.
- `claude_stream.rs` contains Claude stream-json parsing and encoding only.
- Provider-specific trackers live beside their provider adapter.
- Shared contract types contain no native provider enums or JSON field names.
- Neither provider adapter imports the other provider adapter.
- Adding a third harness requires implementing the contract and registering a
  descriptor; baseline CLI and GTK paths require no provider-specific branch.

### Contract Conformance

One table-driven conformance suite runs against deterministic Codex and Claude
fixtures. It proves the required baseline from the Archcar boundary:

- readiness occurs only after native initialization;
- first, queued, and immediate inputs retain identity and complete once;
- canonical and native events are both persisted;
- interrupt settles or resumes deterministically;
- changed controls apply without losing thread affinity;
- crash recovery never replays acknowledged input twice;
- interactions survive refresh and resolve through the same protocol;
- unsupported optional features return structured reasons;
- kill and Archcar shutdown leave no provider descendants.

Provider-specific suites still test native codecs and unusual native events.
Changing the required baseline means changing the contract and its shared
suite first; both Codex and Claude must then pass before the change lands.

## Session And Turn Lifecycle

### Startup

1. `EnsureChatThreadSession` resolves the selected thread and provider kind.
2. Archcar restores the persisted Claude native session ID and desired controls.
3. Archcar launches Claude and records the child process.
4. Startup, plugin, and hook events may arrive before `system/init`; they are
   persisted but do not mark the session ready.
5. `system/init` supplies the native session ID, effective model, tools, MCP
   servers, plugins, and optional runtime capabilities.
6. Archcar persists the native session ID, publishes observed capabilities,
   and emits `SessionReady` once.

An initialization timeout produces a visible session error and terminates the
child. A process is never advertised as ready merely because it was spawned.

### Input Delivery

Archcar gives every local input a durable sequence and delivery state. The
state progresses through queued, written, acknowledged, running, and terminal.

- A normal send while idle is written immediately.
- A normal send while busy remains in Archcar's stable per-thread queue.
- An immediate send is offered to the live provider even while it is busy.
- Claude's streaming input supports provider-side queued messages; Archcar
  still owns the visible durable queue and does not infer delivery from mutable
  GTK state.
- A replayed user message acknowledges the oldest matching written input.
- User transcript content is persisted once and is not duplicated on replay,
  resume, or optimistic GTK refresh.

The queue lookup must include the selected `SessionKind`; no helper may
hardcode `SessionKind::Codex` for a Claude thread.

### Turn Completion

The Claude adapter tracks one logical local turn segment for each delivered
input. Native assistant messages and tool events attach to that segment until a
terminal result arrives.

A terminal Claude result emits exactly one `TurnCompleted` with a normalized
status such as success, failed, interrupted, or deferred. It then emits
`SessionReady` only if the supervisor is able to accept the next turn. Duplicate
or late result records do not complete the same local turn twice.

### Thread Affinity

Every Claude process is launched through `EnsureChatThreadSession`, not the
generic workspace fallback. The Archductor thread stores Claude's native
session ID, and `--resume` uses that ID only for the matching thread and
workspace. Switching between Claude threads must never attach both UI threads
to the first or default Claude process.

## Runtime Controls

The CLI does not expose the Agent SDK's in-process control methods. Archcar
therefore presents the same logical controls through provider-appropriate
mechanics.

### Interrupt

Archcar sends an interrupt to the complete Claude process group and records the
request against the active local turn. If the installed CLI reports an
interrupt receipt, Archcar consumes it. Older supported versions are handled by
observing the result or child exit.

If Claude exits, or fails to settle within a short deadline, Archcar terminates
the child, marks the active segment interrupted, and relaunches with
`--resume`. The conversation survives even when the transport process does not.

The implementation begins with a live transport characterization test because
signal behavior varies by Claude CLI release. The restart-and-resume path is the
required fallback rather than an exceptional manual recovery.

### Model, Effort, And Permission Mode

Claude model, effort, and permission mode are desired session controls stored
by Archcar.

- If Claude is idle, a change performs a controlled child recycle immediately.
- If Claude is busy, the change is recorded and applied before the next queued
  input.
- The new child launches with the same native session ID and the updated flag.
- A failed resume leaves the input queued and reports the failure; it does not
  silently start an unrelated conversation.

GTK exposes Claude model and effort controls in the same locations as Codex
model and thinking controls. Provider-specific labels and supported values may
differ, but the Archcar requests and control state are provider-neutral.

## Permissions, Questions, And Plan Approval

Archductor injects command hooks through the launch's session settings. The hook
command is a hidden Archductor CLI route that reads exactly one Claude hook JSON
object from stdin and writes exactly one Claude hook response JSON object to
stdout. Diagnostics go to stderr so they cannot corrupt hook output.

The hidden command communicates with the existing Archcar daemon. It does not
call Anthropic and does not read credentials.

### Permission Requests

A `PermissionRequest` hook fires only when Claude would show a permission
dialog. The bridge creates a persisted provider interaction, emits an Archcar
interaction event, and waits for a UI or CLI decision within the configured
hook timeout.

The response supports:

- allow once;
- deny with a message;
- allow and apply one of Claude's supplied permission suggestions;
- interrupt after denial when explicitly selected.

Persisted permission changes are applied only when the user selects an explicit
"always" choice. Archductor does not silently write `.claude/settings.json` or
user settings.

### AskUserQuestion And ExitPlanMode

`PreToolUse` hooks match `AskUserQuestion` and `ExitPlanMode`. On the first
invocation, the bridge persists the native tool ID and input and returns
`permissionDecision: "defer"`. Claude exits with `stop_reason:
"tool_deferred"`, preserving the pending call in its transcript.

GTK displays the question or plan without keeping a hook process blocked. After
the user answers, approves, or declines, Archcar persists the resolution and
resumes the same native session. The repeated hook invocation finds the
resolution by native tool ID and returns allow or deny plus the required
`updatedInput`.

Claude documents this defer/resume workflow specifically for custom UIs running
`claude -p`. The adapter treats `tool_deferred_unavailable` and the documented
parallel-tool deferral limitation as visible failures rather than pretending
the interaction succeeded.

### Pending Interaction Persistence

Add a small provider-interaction record with:

- Archductor interaction ID;
- provider, workspace, thread, and session IDs;
- native session and tool IDs when available;
- interaction kind;
- native request JSON;
- pending, allowed, denied, answered, expired, or failed status;
- native response JSON;
- created and resolved timestamps.

This makes question and plan deferrals restart-safe and makes permission
decisions auditable. Sensitive hook values remain local in the existing
database and are not copied into general diagnostic strings.

## Event Normalization

Raw stdout lines are persisted before parsing. The parser then normalizes at
least:

- `system/init` and other system lifecycle records;
- startup hooks and plugin installation events;
- user-message replay acknowledgements;
- assistant messages and final content blocks;
- message and content-block start, delta, and stop events;
- thinking and reasoning content;
- tool use, partial tool input, and tool results;
- file changes and shell commands;
- MCP, skills, plugins, hooks, and subagents;
- permissions, deferred tools, and user questions;
- usage, cost, context, and model metadata;
- `api_retry`, `rate_limit_event`, and other limit failures;
- success, failure, interruption, and deferred results;
- unknown records.

Unknown valid JSON is stored as a canonical unknown provider event with the
native type and payload. Malformed lines are stored in diagnostic output and
produce bounded warnings; they do not crash the stream reader.

Assistant text is assembled without duplicating content from both partial
deltas and the final assistant object. Stable native IDs are preferred for
deduplication, with local per-turn sequence as the fallback.

## Capability Discovery

Replace optimistic static provider declarations with the harness descriptor
plus observed runtime state.

- Required baseline capabilities come from the managed-harness contract and
  conformance suite, not from provider marketing or parser coverage.
- Optional provider extensions use the common support modes and stable reasons.
- `system/init` model, tools, MCP, and plugin fields refine the session state.
- Newer Claude versions may include a `capabilities` array. Archcar
  feature-detects known values and ignores unknown ones.
- Older versions without the field remain supported through the documented
  baseline and transport fallbacks.
- GTK hides or disables a control only when the current session cannot perform
  it, and explains why.

The minimum supported version is Claude Code 2.1.89 because documented
PreToolUse deferral begins there. The current local 2.1.177 remains supported.
Claude Code 2.1.208 or later is recommended for newer stream completion fixes,
but Archductor must tolerate older supported output and must never update the
user's CLI automatically.

## Failure Handling And Recovery

### Preflight

Before launch, Archcar distinguishes:

- executable missing;
- invalid or unsupported version;
- logged-out local Claude account;
- invalid launch configuration;
- startup timeout;
- native resume failure.

Errors name the corrective action without exposing credentials. A doctor or
status command can report the detected executable, version, authentication
state, and supported Archductor capabilities.

### Child Exit

On unexpected exit, Archcar drains remaining stdout, persists the exit code,
and classifies the active input using acknowledgement and terminal-result
state:

- not acknowledged: safe to retain or resend after resume;
- acknowledged with terminal result: do not resend;
- acknowledged without terminal result: mark interrupted or failed and resume
  the conversation without duplicating the prompt.

The user sees a recoverable error when Archcar cannot prove safe automatic
delivery.

### Archcar Restart

Archcar reconciles stale provider processes using the existing managed-session
strategy. It restores the thread's native Claude ID, desired controls, queued
inputs, active delivery state, and pending provider interactions. A new Claude
child resumes the native conversation on demand.

### Rate Limits And Retries

`api_retry` becomes progress with attempt and delay metadata. A terminal rate
limit or billing result becomes a visible limit failure and completes the local
turn once. Archcar does not add an independent network retry around Claude's
own API loop.

## GTK Behavior

- Selecting Claude creates or selects a Claude thread.
- First send uses `EnsureChatThreadSession` and releases only that thread's
  queued prompt after real readiness.
- Queue, busy state, and controls are keyed by thread and `SessionKind`.
- Claude uses the common transcript and provider-event rendering pipeline.
- Model and effort controls appear for Claude with provider-appropriate values.
- Interrupt copy and behavior are provider-neutral.
- Permission, question, and plan interactions appear inline or in a focused
  dialog and remain recoverable after refresh.
- Multiple Claude threads and a simultaneous Codex thread remain isolated.
- Provider errors appear in the affected conversation without breaking the
  rest of the workspace UI.

## CLI Behavior

The CLI can perform every Archcar mutation required by GTK:

- ensure or start a Claude thread session;
- send normal and immediate input;
- inspect status and messages;
- interrupt and kill;
- set model, effort, and permission mode;
- list and resolve pending provider interactions;
- run Claude diagnostics;
- execute the hidden Claude hook bridge route.

Human-facing output remains concise. The hidden hook route emits machine JSON
only.

## Testing Strategy

### Unit Tests

- common managed-harness descriptor and control-plan behavior;
- the same required baseline conformance cases against Codex and Claude
  adapters;
- launch argument construction, including resume, settings, controls, and user
  replay;
- input encoding and replay acknowledgement matching;
- parser fixtures for every supported native event family;
- deduplication of partial and final assistant text;
- result-to-turn status mapping;
- version and capability handling;
- hook input classification and allow, deny, answer, defer, and timeout output;
- provider-interaction persistence and state transitions;
- provider-neutral queue and thread lookup for both Codex and Claude.

### Managed-Process Integration Tests

Use a deterministic fake `claude` executable that speaks the documented JSONL
framing. Cover:

- startup is not ready before `system/init`;
- first and later inputs are acknowledged and completed once;
- queued and immediate input behavior;
- interrupt where the child survives;
- interrupt where the child exits and is resumed;
- model, effort, and permission-mode recycle;
- crash before and after input acknowledgement;
- malformed and unknown output;
- deferred question resume;
- permission bridge approval and denial;
- Archcar restart and stale-process recovery;
- two Claude threads with distinct native IDs.

### CLI Smoke

Run the repository CLI against the deterministic fake provider in automated
tests. Before completion, also run an authenticated local smoke with the
installed Claude CLI:

- preflight and startup;
- one read-only prompt;
- one follow-up on the same native session;
- message projection;
- interrupt and resume;
- a disposable permission or question flow when it can be done safely.

### GTK Smoke

Run written GTK state tests plus a real desktop smoke:

- first Claude send;
- streamed assistant output;
- multiple Claude threads;
- normal queue and immediate input;
- interrupt;
- model and effort change;
- permission and question interaction;
- Archcar restart and resume;
- regression check for Codex and Shell surfaces.

No user-visible behavior is called complete until written tests, relevant CLI
smoke, and relevant GTK smoke pass or the incomplete surface is explicitly
reported.

## Implementation Boundaries

Add the focused `harness_contract.rs` module and shared conformance test helper.
Keep the existing provider adapters, Archcar session manager, protocol,
persistence, CLI, and GTK surface. Add another focused module only where it
keeps the hook bridge or interaction persistence out of the already-large
session loop.

Do not introduce a generic process framework or merge the native session loops.
The shared contract describes observable behavior and pure translation effects;
Codex and Claude retain independent native implementations.

## Collision-Safe Delivery

Other work is actively changing Archcar queueing and the GTK session surface.
Implementation must:

1. begin each slice by re-reading `git status`, the relevant live diff, and the
   latest shared-contract code;
2. preserve concurrent changes and stop if the same hunk cannot be reconciled
   safely;
3. build on the newly provider-neutral queue contract instead of restoring old
   Codex-only helpers;
4. avoid whole-file rewrites, format-only passes, and unrelated cleanup;
5. commit provider-scoped slices with semantic messages;
6. run focused Claude tests plus affected Codex tests after every shared
   protocol or GTK change;
7. never push without an explicit request.

## Non-Goals

- Replacing the installed Claude Code CLI with the Agent SDK.
- Supporting API-key-only or hosted Claude execution.
- Reproducing Claude's interactive terminal renderer.
- Managing the user's Claude login, subscription, auto-updater, or global
  settings.
- Making Codex and Claude native protocols identical.
- Requiring optional provider extensions such as Codex goals to exist on every
  harness.
- Adding abstractions beyond the required managed-harness contract and its
  conformance suite.

## Completion Criteria

Claude is complete when:

- a fresh Claude chat reliably sends and completes its first turn;
- follow-ups, queueing, immediate delivery, interruption, and controls work;
- permissions, questions, and plan approval round-trip through Archcar;
- native and canonical events persist without duplicate transcript content;
- two Claude threads resume their own native conversations;
- Archcar restart does not lose acknowledged input or pending interaction;
- CLI and GTK expose the same behavior;
- Codex and Claude both pass the managed-harness baseline conformance suite;
- optional provider features are capability-gated rather than hardcoded or
  falsely advertised;
- the written unit, integration, CLI, and GTK verification passes;
- Codex and Shell regressions remain green.
