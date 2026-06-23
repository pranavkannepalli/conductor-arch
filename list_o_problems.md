1. takes way too much to create a workspace, requires manual branch name entry whereas conductor just uses random city names + v1 if they're already used and then a later prompt renames it (either on PR or renamed in system prompt - prompt prefix configurable)
2. thread 'main' (189728) panicked at crates/gtk-app/src/terminal.rs:2876:38:
   RefCell already mutably borrowed
   note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread 'main' (189728) panicked at library/core/src/panicking.rs:225:5:
panic in a function that cannot unwind
stack backtrace:
0: 0x5654477fee0a - <<std[ad79df7613b1bf21]::sys::backtrace::BacktraceLock>::print::DisplayBacktrace as core[849aa0802ea38d20]::fmt::Display>::fmt
1: 0x565447815d6a - core[849aa0802ea38d20]::fmt::write
2: 0x565447804102 - <std[ad79df7613b1bf21]::sys::stdio::unix::Stderr as std[ad79df7613b1bf21]::io::Write>::write_fmt
3: 0x5654477dd55f - std[ad79df7613b1bf21]::panicking::default_hook::{closure#0}
4: 0x5654477f7f11 - std[ad79df7613b1bf21]::panicking::default_hook
5: 0x5654477f80cb - std[ad79df7613b1bf21]::panicking::panic_with_hook
6: 0x5654477dd64a - std[ad79df7613b1bf21]::panicking::panic_handler::{closure#0}
7: 0x5654477d4809 - std[ad79df7613b1bf21]::sys::backtrace::**rust_end_short_backtrace::<std[ad79df7613b1bf21]::panicking::panic_handler::{closure#0}, !>
8: 0x5654477de19d - **rustc[9f99ce0b7b54e6bf]::rust_begin_unwind
9: 0x5654478163cd - core[849aa0802ea38d20]::panicking::panic_nounwind_fmt
10: 0x56544781634b - core[849aa0802ea38d20]::panicking::panic_nounwind
11: 0x5654478164d7 - core[849aa0802ea38d20]::panicking::panic_cannot_unwind
12: 0x565446edb5a7 - gtk4::auto::list_box::ListBox::connect_row_selected::row_selected_trampoline::h884e8f3dc475ab93
at /home/kitts/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gtk4-0.11.3/src/auto/list_box.rs:538:9
13: 0x7fdc743b766b - g_cclosure_marshal_VOID**OBJECTv
14: 0x7fdc743dab7a - <unknown>
15: 0x7fdc743dacd9 - g_signal_emit_valist
16: 0x7fdc743dad94 - g_signal_emit
17: 0x7fdc739897cd - <unknown>
18: 0x7fdc7398bcc4 - <unknown>
19: 0x7fdc7388faa8 - <unknown>
20: 0x7fdc743dab7a - <unknown>
21: 0x7fdc743dacd9 - g_signal_emit_valist
22: 0x7fdc743dad94 - g_signal_emit
23: 0x7fdc73950bb5 - <unknown>
24: 0x7fdc743bce63 - g_cclosure_marshal_VOID**BOXEDv
25: 0x7fdc743dab7a - <unknown>
26: 0x7fdc743dacd9 - g_signal_emit_valist
27: 0x7fdc743dad94 - g_signal_emit
28: 0x7fdc7395376e - <unknown>
29: 0x7fdc73955efb - <unknown>
30: 0x7fdc73956cbb - <unknown>
31: 0x7fdc73ae102b - <unknown>
32: 0x7fdc7399cc72 - <unknown>
33: 0x7fdc7399d7c3 - <unknown>
34: 0x7fdc738948db - <unknown>
35: 0x7fdc73d5a348 - <unknown>
36: 0x7fdc743b8bcc - g_closure_invoke
37: 0x7fdc743d884b - <unknown>
38: 0x7fdc743da294 - <unknown>
39: 0x7fdc743dacd9 - g_signal_emit_valist
40: 0x7fdc743dad94 - g_signal_emit
41: 0x7fdc73d60832 - <unknown>
42: 0x7fdc73cb66fc - <unknown>
43: 0x7fdc73510bfd - <unknown>
44: 0x7fdc73512e57 - <unknown>
45: 0x7fdc73512fe5 - g_main_context_iteration
46: 0x7fdc736fbc36 - g_application_run
47: 0x565446f1c179 - gio::application::ApplicationExtManual::run_with_args::hdeaa398d92a3a8cf
at /home/kitts/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gio-0.22.6/src/application.rs:25:13
48: 0x565446f1c349 - gio::application::ApplicationExtManual::run::h58a27c45fa716857
at /home/kitts/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/gio-0.22.6/src/application.rs:17:14
49: 0x565446ef9a95 - linux_conductor_gtk::main::h60e4f8cb59ee0bf0
at /home/kitts/Documents/dev/personal/conductor-arch/crates/gtk-app/src/main.rs:173:9
50: 0x565446fb1f4b - core::ops::function::FnOnce::call_once::hd078fa94493cf1ca
at /usr/src/debug/rust/rustc-1.96.0-src/library/core/src/ops/function.rs:250:5
51: 0x565446f2b2ce - std::sys::backtrace::**rust_begin_short_backtrace::hf2537353c026df1b
at /usr/src/debug/rust/rustc-1.96.0-src/library/std/src/sys/backtrace.rs:166:18
52: 0x565446fe9bd1 - std::rt::lang_start::{{closure}}::h08e12f942a624bb0
at /usr/src/debug/rust/rustc-1.96.0-src/library/std/src/rt.rs:206:18
53: 0x5654477f6d44 - std[ad79df7613b1bf21]::rt::lang_start_internal
54: 0x565446fe9bb7 - std::rt::lang_start::h417d93f8c573d42b
at /usr/src/debug/rust/rustc-1.96.0-src/library/std/src/rt.rs:205:5
55: 0x565446efd4de - main
56: 0x7fdc73027741 - <unknown>
57: 0x7fdc73027879 - **libc_start_main
58: 0x565446e57fb5 - \_start
59: 0x0 - <unknown>
thread caused non-unwinding panic. aborting.
make: \*\*\* [Makefile:18: gtk] Aborted (core dumped)

3. takes wayyy too much to create add a repo. these things are almost one click in conductor - click the plus button, opens a modal (no projects tab), modal has 3 options (folder - opens folder selector, clone - opens selector for gh repos, new - select from 3 template options and folder selector for where to put the new project)

4) smoother settings surface with better formatted text fields and shit.

5) filters + refine all buttons (they look ai generated right now)

6) Doesn't align with design schema:
   Design language — "Conductor-style" dark IDE shell

Vibe: Calm, dense, developer-native. Dark, low-contrast surfaces with one green accent. Nothing decorative — every pixel is information or affordance. Think "native desktop app," not "web page." No gradients, no rounded cards-with-left-border-accent, no shadows on a dark UI.

Surfaces (elevation by lightness, not borders/shadows):

    App background / chat column: #181a18 (darkest)
    Sidebar: #1e201f (slightly raised)
    Right/diff panel: #15181b
    Title bar: #1c201d
    Hover/selected fill: #2a2e2c → #2c2f2c
    Hairline dividers: #2a2c2a (barely visible — separation comes from lightness, not lines)

Text (3 levels only):

    Primary: #e4e8e4
    Secondary: #b4b8b4
    Muted (file paths, metadata, placeholders): #8a8f88

Accent: one green, #3fb950. Used only for status/affirmative meaning — run dot, "ready for review," success log lines. Never for decoration. Destructive = #c2422d (close-button hover only).

Type:

    UI: Inter, 13–15px. Weights: 400 body, 500 labels, 600 active/headers.
    Code, file paths, terminal, keyboard keys: JetBrains Mono.
    That mono/sans split is the core signal: anything the machine owns is monospace.

Spacing & shape:

    7px radius on interactive chips/rows/buttons; 14px on large containers (composer, bubbles).
    Tight, consistent gaps (gap: 4–18px in flex rows). Rows ~8–9px vertical padding.
    Title/tab bars are a fixed ~46–49px.

Components:

    Tabs: text + 15px icon, active = primary color + 2px bottom border, inactive = muted.
    List rows: full-width, hover-fill, selected = solid fill (no border).
    Status pills: filled, mono or semibold, color carries meaning.
    Layout is always flex/grid with gap — never margin-spaced inline siblings.

Principles to hold the LLM to:

    Separate surfaces by lightness steps, not borders or shadows.
    One accent color, semantic only.
    Three text shades, no more.
    Monospace = machine, sans = human.
    Information density over whitespace, but never cramped — let lightness do the work.
    No AI-slop: no gradients, glow, emoji (the train mark is the one intentional brand exception), or rounded-accent-border cards.

Want me to drop this into a DESIGN_SYSTEM.md in the project, or generate a token table (CSS variables / a Rust theme struct) you can wire directly into your app?
