class Archductor < Formula
  desc "Parallel coding-agent workflow tool built around Git worktrees"
  homepage "https://github.com/perceo-ai/conductor-arch"
  url "https://github.com/perceo-ai/conductor-arch/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "3ce40f320f273a5038d818e071671c9c65c1c3c22662166c5dbfbee3b5795022"
  license "Apache-2.0"

  depends_on :linux
  depends_on "pkgconf" => :build
  depends_on "rust" => :build
  depends_on "gh"
  depends_on "git"
  depends_on "gtk4"
  depends_on "libadwaita"
  depends_on "sqlite"

  def install
    ENV["LIBSQLITE3_SYS_USE_PKG_CONFIG"] = "1"
    system "cargo", "build", "--release", "--locked", "--workspace"
    bin.install "target/release/archductor"
    bin.install "target/release/archductor-gtk"
    bin.install "target/release/archcar"
    pkgshare.install "README.md"
    share.install "packaging/archductor-gtk.desktop"
    share.install "packaging/archductor.svg"
  end

  test do
    system "#{bin}/archductor", "doctor"
  end
end
