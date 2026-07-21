# Interactive `make dev` Design

## Goal

Make `make dev` behave like Flutter's interactive development command on
native Windows and Linux. Source changes do not trigger automatic rebuilds.
The developer explicitly reloads GTK with `r` and exits with `q`.

## Terminal Contract

After the initial workspace build, `make dev` starts Archcar and GTK and prints
the available controls:

- `r`: rebuild and restart GTK. Archcar remains running.
- `q`: stop GTK, Archcar, and every descendant process owned by this invocation,
  then exit successfully.
- `Ctrl+C`: perform the same owned-process cleanup as `q` and exit as an
  interrupted command.

Other input is ignored. Input is handled one key at a time without requiring
Enter when the terminal supports raw key input.

## Architecture

A small Rust development-runner binary owns the interactive loop and child
process lifecycle on every platform. The Makefile invokes this binary through
the existing platform environment wrapper. This replaces Cargo Watch and the
separate Windows PowerShell lifecycle runner, preventing Windows and Arch Linux
from developing different controls.

The runner launches `cargo run --bin archcar` once. It launches GTK with
`cargo run --bin archductor-gtk`; an `r` request terminates that GTK process
tree, waits for cleanup, and launches it again. A `q`, interrupt, child failure,
or runner failure terminates both owned process trees. Cleanup is scoped to
children created by the current runner and never searches by executable name.

## Failure Handling

If the initial build fails, no long-running children start. If Archcar exits
unexpectedly, the runner stops GTK and exits nonzero. If a GTK build or launch
fails during reload, the runner reports the error while retaining ownership of
Archcar and continues accepting `r` or `q`, allowing the developer to fix the
source and retry.

## Verification

Written tests cover key parsing, reload state transitions, quit behavior, and
Makefile wiring without relying on a real terminal. CLI smoke starts the runner
and exercises `r` and `q` against controlled child commands. GTK smoke confirms
the real GTK binary launches, reloads only after `r`, and leaves no owned
processes after `q`.
