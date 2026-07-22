# Packaging

Packaging is not the product-readiness gate. These commands validate release
artifacts, but public release readiness depends on the app workflow in
`docs/manual-testing-checklist.md`.

Do not call packaging release-ready until the app workflow has been manually
validated on the announced platform: repository onboarding/settings, workspace creation,
embedded agent sessions, terminal/runtime, diff/review/checks/todos, PR
merge/archive, history, provider/MCP status, customization settings, and known
gaps documented. Native Windows now has intentional process, PTY, path, shell,
IPC, compile, and ZIP packaging boundaries, but the ZIP remains preview-only
until its real Windows checklist passes. macOS is not a release target.

## Build packages locally

## Typography assets

The GTK theme uses the bundled `Mona Sans` variable font for UI text and
`Commit Mono` for code-like surfaces. Native packages install these fonts into
their platform font-data location; the Windows portable bundle loads its font
directory privately when the app starts.

Do not replace the app CSS with `Inter`, `Geist`, or other generic SaaS font
defaults when packaging. The runtime CSS already includes the fallback stacks
needed for systems where Mona Sans or Commit Mono are not installed.

Install [nfpm](https://nfpm.goreleaser.com/):

```bash
# Linux
curl -fsSL https://github.com/goreleaser/nfpm/releases/download/v2.38.0/nfpm_2.38.0_Linux_x86_64.tar.gz \
  | tar -xz -C /usr/local/bin nfpm
```

Build the release binary first:

```bash
cargo build --workspace --release --locked
```

Or run the local release-readiness gate from the repository root. Package mode
only creates artifacts on Linux; on other platforms, use the tag-driven CI
workflow for Linux artifacts.

```bash
scripts/release-readiness.sh --version 0.1.0
scripts/release-readiness.sh --version 0.1.0 --package
```

### .deb (Ubuntu / Debian)

```bash
VERSION=0.1.0 nfpm package --packager deb --target dist/
sudo dpkg -i dist/archductor_0.1.0_amd64.deb
```

### .rpm (Fedora / openSUSE)

```bash
VERSION=0.1.0 nfpm package --packager rpm --target dist/
sudo rpm -i dist/archductor-0.1.0-1.x86_64.rpm
```

### AppImage

```bash
# Install appimagetool
curl -fsSL -o /usr/local/bin/appimagetool \
  https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x /usr/local/bin/appimagetool

# Copy binaries into AppDir
install -Dm755 target/release/archductor \
  packaging/appimage/archductor.AppDir/usr/bin/archductor
install -Dm755 target/release/archductor-gtk \
  packaging/appimage/archductor.AppDir/usr/bin/archductor-gtk

# Build AppImage
appimagetool --appimage-extract-and-run \
  packaging/appimage/archductor.AppDir \
  dist/archductor-0.1.0-x86_64.AppImage
```

With no arguments, the AppImage launches the GTK GUI. With arguments, it passes
through to the CLI, for example:

```bash
./dist/archductor-0.1.0-x86_64.AppImage doctor
```

### AUR (Arch Linux)

```bash
scripts/update-aur-checksum.sh 0.1.0 <64-character-sha256>
cd packaging/aur
makepkg -si
```

### Nix

```bash
nix build
nix run .#archductor -- doctor
nix run .#archductor-gtk
```

Use `nix develop` for a Rust + GTK development shell.

### Homebrew Tap (Linuxbrew)

```bash
brew tap perceo-ai/tap
brew install archductor
archductor doctor
```

The formula source lives in `packaging/homebrew/Formula/archductor.rb`; the
tag-driven publish workflow copies it to `perceo-ai/homebrew-tap`.

### Flatpak (experimental)

The Flatpak build requires `flatpak-builder`, the GNOME 50 SDK, and the Rust SDK
extension:

```bash
flatpak install flathub org.gnome.Platform//50 org.gnome.Sdk//50
flatpak install flathub org.freedesktop.Sdk.Extension.rust-stable//25.08

# Refresh Rust crate sources after Cargo.lock changes
flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json

# Build and install locally
flatpak-builder --install --user --force-clean \
  build-dir \
  packaging/flatpak/ai.perceo.Archductor.yml

# Run
flatpak run ai.perceo.Archductor
```

> **Note:** The Flatpak sandbox requires `--filesystem=host` to access arbitrary
> repository paths. The app works best installed from AppImage or native packages.

### Windows portable ZIP (preview)

The tag workflow builds `archductor-<version>-windows-x86_64.zip` with the CLI,
GTK app, archcar sidecar, GTK/libadwaita DLLs, loaders, schemas, icons, and MIME
data. Extract the full archive before launching `archductor-gtk.exe`; do not
copy only the executable. The workflow also emits `SHA256SUMS-windows.txt`.

Before promotion beyond preview, validate extraction, GUI launch, CLI doctor,
project/workspace creation, PTY sessions, provider sessions, stop/restart,
upgrade-over-extracted-install, and checksum verification on real Windows.

## CI release

Push a tag to trigger the publish workflow:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The workflow builds tarball + .deb + .rpm + AppImage on `ubuntu-24.04` and the
portable Windows ZIP on `windows-2025`, then attaches the artifacts to the
GitHub release.

For a manual dry run from the Actions UI, run the `Publish` workflow with an
explicit version such as `0.1.0`. Manual dispatch does not create a GitHub
release unless it runs on a tag, but it still builds and uploads package
artifacts for inspection.

Release readiness requires more than attached artifacts. Before a public
release, add or verify publishing pipelines for every supported Linux package
channel: AppImage/GitHub releases, `.deb` repositories for APT, `.rpm`
repositories for DNF or zypper, AUR, Flatpak, Nix, and Homebrew. Each channel
should be tag-driven where supported, publish checksums where supported, and
have documented rollback or yanking steps.

The release pipeline must also build the website as a subset of `perceo.ai`, or
treat that website build as a required release gate. The site should include
release downloads, install instructions, supported Linux targets, known limits,
and links to the GitHub release artifacts.
