{
  description = "Archductor parallel coding-agent workflow tool";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          archductor = pkgs.rustPlatform.buildRustPackage {
            pname = "archductor";
            version = "0.1.0";

            src = pkgs.lib.cleanSource ./.;
            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = with pkgs; [
              pkg-config
              wrapGAppsHook4
            ];

            buildInputs = with pkgs; [
              gtk4
              libadwaita
              sqlite
            ];

            LIBSQLITE3_SYS_USE_PKG_CONFIG = "1";
            cargoBuildFlags = [ "--workspace" ];
            doCheck = false;

            installPhase = ''
              runHook preInstall

              install -Dm755 target/release/archductor "$out/bin/archductor"
              install -Dm755 target/release/archductor-gtk "$out/bin/archductor-gtk"
              install -Dm755 target/release/archcar "$out/bin/archcar"
              install -Dm644 packaging/archductor-gtk.desktop \
                "$out/share/applications/archductor-gtk.desktop"
              install -Dm644 packaging/archductor.svg \
                "$out/share/icons/hicolor/scalable/apps/archductor.svg"
              install -Dm644 README.md "$out/share/doc/archductor/README.md"

              runHook postInstall
            '';

            meta = with pkgs.lib; {
              description = "Parallel coding-agent workflow tool built around Git worktrees";
              homepage = "https://github.com/perceo-ai/conductor-arch";
              license = licenses.asl20;
              mainProgram = "archductor";
              platforms = platforms.linux;
            };
          };
        in
        {
          inherit archductor;
          default = archductor;
        });

      apps = forAllSystems (system: {
        archductor = {
          type = "app";
          program = "${self.packages.${system}.archductor}/bin/archductor";
        };
        archductor-gtk = {
          type = "app";
          program = "${self.packages.${system}.archductor}/bin/archductor-gtk";
        };
        default = self.apps.${system}.archductor;
      });

      devShells = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              pkg-config
              gtk4
              libadwaita
              git
              gh
              sqlite
              openssh
            ];
          };
        });

      formatter = forAllSystems (system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        pkgs.nixpkgs-fmt);
    };
}
