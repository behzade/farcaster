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
    in
    {
      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          nativeRuntimeLibraries =
            with pkgs;
            lib.optionals stdenv.isLinux [
              alsa-lib
              at-spi2-atk
              cairo
              cups
              dbus
              expat
              file
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
            ];
            shellHook = pkgs.lib.optionalString pkgs.stdenv.isLinux ''
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
