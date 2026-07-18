# Release Readiness

This runbook is the launch gate for Archductor. The Rust workspace can be
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
build, `archductor doctor`, and `cargo deny check` when `cargo-deny` is
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

- `archductor-<version>-linux-x86_64.tar.gz`
- `.deb`
- `.rpm`
- AppImage
- `archductor-<version>-windows-x86_64.zip`
- `SHA256SUMS`
- `SHA256SUMS-windows.txt`
- Trivy release-artifact scan result
- provenance attestations
- GitHub release attachments for tag runs
- AUR package update when `AUR_SSH_PRIVATE_KEY` is configured
- Homebrew tap update when `HOMEBREW_TAP_TOKEN` is configured

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
  defaults, keybindings, terminal font/scrollback, configured check scripts,
  prompt pack metadata, and command presets

Linear live validation requires `LINEAR_API_KEY`. GitHub validation requires
`gh auth status` to be authenticated.

## Package Channel Gate

Do not announce support for a channel until install, upgrade, launch, checksum,
and rollback or yank paths are validated for that channel.

| Channel | Launch Requirement |
| --- | --- |
| GitHub/AppImage | Tag workflow attaches AppImage and checksum; AppImage opens GUI with no args and forwards CLI args. |
| Debian/Ubuntu | `.deb` installs with `dpkg` or `apt`, launches GUI, runs `archductor doctor`, and has upgrade/removal notes. |
| Fedora/openSUSE | `.rpm` installs with `rpm`, `dnf`, or `zypper`, launches GUI, runs `archductor doctor`, and has upgrade/removal notes. |
| AUR | `PKGBUILD` uses the release tag and real checksum, `makepkg -si` passes on Arch, and update/yank process is documented. |
| Flatpak/Flathub | `packaging/flatpak/ai.perceo.Archductor.yml` builds locally, metadata validates, screenshots are current, Flathub review accepts the broad filesystem access requirement, and the Flathub package launches the GUI. |
| Nix | `nix build` and `nix run .#archductor -- doctor` pass on Linux, and the flake is referenced from install docs before a nixpkgs submission. |
| Homebrew | `perceo-ai/homebrew-tap` formula installs on Linuxbrew, `brew test archductor` passes, and tag publish refreshes the formula checksum. |
| Windows ZIP | Archive contains all GTK runtime files, extracts cleanly, launches GUI and CLI, verifies checksum, upgrades safely, and passes the Windows workflow checklist. Preview until proven on real Windows. |

## Website Gate

The `perceo.ai` Archductor page must ship before public launch or be a
required release gate. It needs:

- Archductor product page with the real workflow
- download/install instructions for supported channels only
- supported Linux targets and prerequisites
- known limits copied from `progress.md`
- GitHub release links
- checksum and provenance verification instructions
- local build instructions linked back to this repository

## AUR Checksum Update

The `Publish` workflow updates the AUR package on tag pushes after the Linux
release package job passes. It expects the repository secret
`AUR_SSH_PRIVATE_KEY` to contain an SSH private key whose public key is
registered on the AUR account that owns `archductor`.

For manual updates after the release source tarball checksum is known, update
the AUR package:

```bash
scripts/update-aur-checksum.sh 0.1.0 <64-character-sha256>
```

Then install and smoke the AUR package on Arch before publishing:

```bash
cd packaging/aur
makepkg -si
archductor doctor
gtk_status=0
xvfb-run -a timeout 15s archductor-gtk --page dashboard || gtk_status=$?
test "$gtk_status" -eq 0 -o "$gtk_status" -eq 124
```

AUR publishes from a Git repository at
`ssh://aur@aur.archlinux.org/archductor.git`; commit `PKGBUILD` and `.SRCINFO`
to its `master` branch.

## Homebrew Tap

The `Publish` workflow updates `perceo-ai/homebrew-tap` on tag pushes after the
Linux release package job passes. It expects `HOMEBREW_TAP_TOKEN` to contain a
GitHub token with write access to the tap repository.

For manual updates:

```bash
scripts/update-homebrew-formula.sh 0.1.0 <64-character-sha256>
cp packaging/homebrew/Formula/archductor.rb ../homebrew-tap/Formula/archductor.rb
```

Then install, audit, test, and smoke the formula on Linuxbrew before publishing:

```bash
brew audit --strict --online --formula packaging/homebrew/Formula/archductor.rb
brew install --build-from-source packaging/homebrew/Formula/archductor.rb
brew test archductor
archductor doctor
gtk_status=0
xvfb-run -a timeout 15s archductor-gtk --page dashboard || gtk_status=$?
test "$gtk_status" -eq 0 -o "$gtk_status" -eq 124
```

After the checks pass, commit and push the tap repository.

## Nix

The flake exposes:

```bash
nix build
nix run .#archductor -- doctor
nix run .#archductor-gtk
nix develop
```

Run these on Linux before advertising Nix as validated.

## Flathub

The upstream Flatpak files use app ID `ai.perceo.Archductor`:

```bash
appstreamcli validate --explain packaging/flatpak/ai.perceo.Archductor.metainfo.xml
flatpak-builder --force-clean build-dir packaging/flatpak/ai.perceo.Archductor.yml
```

Before submitting to Flathub, add current PNG screenshots, verify the
`perceo.ai` domain/app ID ownership path, and document the need for
`--filesystem=host` because Archductor opens arbitrary repository paths.

## Known Launch Limits

Keep these visible in release notes and the website:

- terminal rendering handles common ANSI/control redraws but is not a full
  terminal emulator
- project onboarding/settings still need polish, but the main Settings surface
  now exposes General, Prompts, Scripts, Git, Terminal, Shortcuts,
  Notifications, Advanced
- deeper layout/theme coverage is incomplete
- prompt pack switching/import/export, naming templates, hooks, local check
  runner UI, and richer notification options are not fully surfaced in the GUI
- visual parity with macOS Archductor is incomplete
- Linux is the primary validated package target; the native Windows ZIP is
  preview-only until its real-machine gate passes
