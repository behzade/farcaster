{
  description = "Farcaster native coding-agent client";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs { inherit system; };
      nonoVersion = "0.74.0";
      nonoReleases = {
        aarch64-darwin = {
          target = "aarch64-apple-darwin";
          hash = "sha256-iOb3FvK7M0lzEFeZt3Tg/ObW1p4AXhpY8SdTxCrPC58=";
        };
        aarch64-linux = {
          target = "aarch64-unknown-linux-gnu";
          hash = "sha256-yDGR4wO2uoyATT6GSRdFWFhPNY0CYB0EbBTkdusIEZw=";
        };
        x86_64-linux = {
          target = "x86_64-unknown-linux-gnu";
          hash = "sha256-tiU5nv1ISQ1jZ6YVnlJBJjqnrC5GTSGm7CnZ6/v91LM=";
        };
      };
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          nonoRelease = nonoReleases.${system};
          nono = pkgs.stdenvNoCC.mkDerivation {
            pname = "nono";
            version = nonoVersion;
            src = pkgs.fetchurl {
              url = "https://github.com/nolabs-ai/nono/releases/download/v${nonoVersion}/nono-v${nonoVersion}-${nonoRelease.target}.tar.gz";
              inherit (nonoRelease) hash;
            };
            dontUnpack = true;
            nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.autoPatchelfHook ];
            buildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.stdenv.cc.cc.lib ];
            installPhase = ''
              mkdir -p "$out/bin"
              tar -xzf "$src" -C "$out/bin" nono
            '';
          };
          nativeRuntimeLibraries =
            with pkgs;
            lib.optionals stdenv.isLinux [
              alsa-lib
              at-spi2-atk
              cairo
              cups
              dbus
              expat
              fontconfig
              freetype
              glib
              gtk3
              libdrm
              libgbm
              libGL
              libcxx
              libxml2
              libx11
              libxcb
              libxcomposite
              libxdamage
              libxext
              libxfixes
              libxkbcommon
              libxkbfile
              libxrandr
              libxshmfence
              nspr
              nss
              pango
              pciutils
              stdenv.cc.cc
              systemd
              vulkan-loader
              wayland
            ];
        in
        {
          default = pkgs.mkShell {
            buildInputs = nativeRuntimeLibraries;
            packages = with pkgs; [
              cargo
              clippy
              git
              neovim
              pkg-config
              rust-analyzer
              rustc
              rustfmt
              rustPlatform.bindgenHook
              zig_0_16
            ] ++ [ nono ];
            shellHook = ''
              export FARCASTER_NONO_PATH="${nono}/bin/nono"
            '' + pkgs.lib.optionalString pkgs.stdenv.isLinux ''
              export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath nativeRuntimeLibraries}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
              export NIX_LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath nativeRuntimeLibraries}''${NIX_LD_LIBRARY_PATH:+:$NIX_LD_LIBRARY_PATH}"
              export NIX_LDFLAGS="-rpath ${pkgs.lib.makeLibraryPath nativeRuntimeLibraries} ''${NIX_LDFLAGS:-}"
            '';
          };
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt-tree);
    };
}
