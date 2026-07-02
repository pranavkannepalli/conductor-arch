# Packaging

Packaging is not the product-readiness gate. These commands validate release
artifacts, but public release readiness depends on the app workflow in
`docs/manual-testing-checklist.md`.

Do not call packaging release-ready until the Linux app workflow has been
manually validated: repository onboarding/settings, workspace creation,
embedded agent sessions, terminal/runtime, diff/review/checks/todos, PR
merge/archive, history, provider/MCP status, customization settings, and known
gaps documented. Windows and macOS packages are not release targets until the
process, PTY, path, and shell abstractions are intentionally ported.

## Build packages locally

## Typography assets

The GTK theme prefers `Mona Sans` for UI text and `Commit Mono` for code-like
surfaces, then falls back to the platform UI and mono stacks. Native packages do
not vendor those font files yet, so release builds should either bundle the
licensed font files into the package target or document them as recommended
desktop fonts for the distribution channel.

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
sudo dpkg -i dist/linux-archductor_0.1.0_amd64.deb
```

### .rpm (Fedora / openSUSE)

```bash
VERSION=0.1.0 nfpm package --packager rpm --target dist/
sudo rpm -i dist/linux-archductor-0.1.0-1.x86_64.rpm
```

### AppImage

```bash
# Install appimagetool
curl -fsSL -o /usr/local/bin/appimagetool \
  https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x /usr/local/bin/appimagetool

# Copy binaries into AppDir
install -Dm755 target/release/linux-archductor \
  packaging/appimage/linux-archductor.AppDir/usr/bin/linux-archductor
install -Dm755 target/release/linux-archductor-gtk \
  packaging/appimage/linux-archductor.AppDir/usr/bin/linux-archductor-gtk

# Build AppImage
appimagetool --appimage-extract-and-run \
  packaging/appimage/linux-archductor.AppDir \
  dist/linux-archductor-0.1.0-x86_64.AppImage
```

With no arguments, the AppImage launches the GTK GUI. With arguments, it passes
through to the CLI, for example:

```bash
./dist/linux-archductor-0.1.0-x86_64.AppImage doctor
```

### AUR (Arch Linux)

```bash
scripts/update-aur-checksum.sh 0.1.0 <64-character-sha256>
cd packaging/aur
makepkg -si
```

### Flatpak (experimental)

The Flatpak build requires `flatpak-builder`, the GNOME 47 SDK, and the Rust SDK
extension:

```bash
flatpak install flathub org.gnome.Platform//47 org.gnome.Sdk//47
flatpak install flathub org.freedesktop.Sdk.Extension.rust-stable//24.08

# Build and install locally
flatpak-builder --install --user --force-clean \
  build-dir \
  packaging/flatpak/io.github.pranavkannepalli.linux-archductor.yml

# Run
flatpak run io.github.pranavkannepalli.linux-archductor
```

> **Note:** The Flatpak sandbox requires `--filesystem=host` to access arbitrary
> repository paths. The app works best installed from AppImage or native packages.

## CI release

Push a tag to trigger the publish workflow:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The workflow builds tarball + .deb + .rpm + AppImage on `ubuntu-24.04` GitHub
hosted runners and attaches all artifacts to the GitHub release.

For a manual dry run from the Actions UI, run the `Publish` workflow with an
explicit version such as `0.1.0`. Manual dispatch does not create a GitHub
release unless it runs on a tag, but it still builds and uploads package
artifacts for inspection.

Release readiness requires more than attached artifacts. Before a public
release, add or verify publishing pipelines for every supported Linux package
channel: AppImage/GitHub releases, `.deb` repositories for APT, `.rpm`
repositories for DNF or zypper, AUR, and Flatpak. Each channel should be
tag-driven, publish checksums where supported, and have documented rollback or
yanking steps.

The release pipeline must also build the website as a subset of `perceo.ai`, or
treat that website build as a required release gate. The site should include
release downloads, install instructions, supported Linux targets, known limits,
and links to the GitHub release artifacts.
