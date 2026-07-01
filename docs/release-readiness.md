# Release Readiness

This runbook is the launch gate for Linux Archductor. The Rust workspace can be
locally healthy while the public launch is still blocked by package-channel,
website, or live-provider validation.

## Local Gate

Run this before cutting a release candidate:

```bash
scripts/release-readiness.sh --version 0.1.0
tests/release-readiness-test.sh
```

For local artifact smoke on Linux when `nfpm` and `appimagetool` are installed:

```bash
scripts/release-readiness.sh --version 0.1.0 --package
```

The local gate covers formatting, clippy, the workspace test suite, release
build, `linux-archductor doctor`, and `cargo deny check` when `cargo-deny` is
available. The focused shell test covers script argument validation and
non-Linux package behavior. On Linux, the package option requires `nfpm` and
`appimagetool`, then creates a tarball, `.deb`, `.rpm`, AppImage, and
`dist/SHA256SUMS`. On other platforms, package mode exits after reporting that
Linux artifacts must be built on Linux or by CI.

## CI Release Gate

Use the `Publish` workflow for authoritative artifacts. Tag pushes derive the
version from the tag. Manual workflow dispatch requires an explicit semantic
version input.

```bash
git tag v0.1.0
git push origin v0.1.0
```

The release workflow must produce:

- `linux-archductor-<version>-linux-x86_64.tar.gz`
- `.deb`
- `.rpm`
- AppImage
- `SHA256SUMS`
- Trivy release-artifact scan result
- provenance attestations
- GitHub release attachments for tag runs

## Manual App Gate

Complete `docs/manual-testing-checklist.md` on a real Linux desktop before
calling a public artifact launch-ready. At minimum, the pass needs:

- repository add and clone from the Projects page
- branch, prompt, GitHub issue, GitHub PR, and Linear workspace creation
- Shell, Codex, Claude Code, and Cursor session launches where CLIs are
  installed and authenticated
- setup/run/stop scripts and terminal transcript behavior
- diff, local review comments, todos, conflicts, PR checks, PR comments,
  review-thread actions, merge, archive, restore, and history
- `.worktreeinclude`, monorepo working directory, linked directories, view
  defaults, keybindings, terminal font/scrollback, and command presets

Linear live validation requires `LINEAR_API_KEY`. GitHub validation requires
`gh auth status` to be authenticated.

## Package Channel Gate

Do not announce support for a channel until install, upgrade, launch, checksum,
and rollback or yank paths are validated for that channel.

| Channel | Launch Requirement |
| --- | --- |
| GitHub/AppImage | Tag workflow attaches AppImage and checksum; AppImage opens GUI with no args and forwards CLI args. |
| Debian/Ubuntu | `.deb` installs with `dpkg` or `apt`, launches GUI, runs `linux-archductor doctor`, and has upgrade/removal notes. |
| Fedora/openSUSE | `.rpm` installs with `rpm`, `dnf`, or `zypper`, launches GUI, runs `linux-archductor doctor`, and has upgrade/removal notes. |
| AUR | `PKGBUILD` uses the release tag and real checksum, `makepkg -si` passes on Arch, and update/yank process is documented. |
| Flatpak | Build result is documented. If published, note broad filesystem access for arbitrary repository paths. |

## Website Gate

The `perceo.ai` Linux Archductor page must ship before public launch or be a
required release gate. It needs:

- Linux Archductor product page with the real workflow
- download/install instructions for supported channels only
- supported Linux targets and prerequisites
- known limits copied from `progress.md`
- GitHub release links
- checksum and provenance verification instructions
- local build instructions linked back to this repository

## AUR Checksum Update

After the release source tarball checksum is known, update the AUR package:

```bash
scripts/update-aur-checksum.sh 0.1.0 <64-character-sha256>
```

Then run `makepkg -si` from `packaging/aur` on Arch before publishing.

## Known Launch Limits

Keep these visible in release notes and the website:

- terminal rendering handles common ANSI/control redraws but is not a full
  terminal emulator
- project onboarding/settings need more polish and fuller visibility
- deeper layout/theme coverage is incomplete
- prompt packs, naming templates, hooks, import/export, and richer notification
  options are not fully surfaced in the GUI
- visual parity with macOS Archductor is incomplete
- Linux is the only launch target for packages
