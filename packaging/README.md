# Packaging

## Build packages locally

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

### .deb (Ubuntu / Debian)

```bash
VERSION=0.1.0 nfpm package --packager deb --target dist/
sudo dpkg -i dist/linux-conductor_0.1.0_amd64.deb
```

### .rpm (Fedora / openSUSE)

```bash
VERSION=0.1.0 nfpm package --packager rpm --target dist/
sudo rpm -i dist/linux-conductor-0.1.0-1.x86_64.rpm
```

### AppImage

```bash
# Install appimagetool
curl -fsSL -o /usr/local/bin/appimagetool \
  https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x /usr/local/bin/appimagetool

# Copy binaries into AppDir
install -Dm755 target/release/linux-conductor \
  packaging/appimage/linux-conductor.AppDir/usr/bin/linux-conductor
install -Dm755 target/release/linux-conductor-gtk \
  packaging/appimage/linux-conductor.AppDir/usr/bin/linux-conductor-gtk

# Build AppImage
appimagetool --appimage-extract-and-run \
  packaging/appimage/linux-conductor.AppDir \
  dist/linux-conductor-0.1.0-x86_64.AppImage
```

With no arguments, the AppImage launches the GTK GUI. With arguments, it passes
through to the CLI, for example:

```bash
./dist/linux-conductor-0.1.0-x86_64.AppImage doctor
```

### AUR (Arch Linux)

```bash
cd packaging/aur
# Update pkgver and sha256sums, then:
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
  packaging/flatpak/io.github.pranavkannepalli.linux-conductor.yml

# Run
flatpak run io.github.pranavkannepalli.linux-conductor
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
