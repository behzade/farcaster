{
  lib,
  rustPlatform,
  callPackage,
  pkg-config,
  cmake,
  makeWrapper,
  writeShellScript,
  zig_0_16,
  llvmPackages_21,
  fontconfig,
  freetype,
  libGL,
  libx11,
  libxcb,
  libxkbcommon,
  libxml2,
  wayland,
  vulkan-loader,
  git,
  neovim,
  cacert,
}:
let
  manifest = builtins.fromTOML (builtins.readFile ../Cargo.toml);
  lock = builtins.fromTOML (builtins.readFile ../Cargo.lock);
  ghostty = builtins.head (builtins.filter (p: p.name == "gpui-libghostty") lock.package);
  zigDeps = callPackage ./ghostty-zig-deps.nix { };
  # --system supplies offline packages but also defaults to system libraries.
  # Keep Ghostty's normal bundled choices, including fontconfig's libxml2,
  # so its static archive retains the linkage expected by the Rust build script.
  ghosttyZig = writeShellScript "farcaster-ghostty-zig" ''
    if [ "''${1-}" = build ]; then
      shift
      exec ${zig_0_16}/bin/zig build \
        -fno-sys=freetype \
        -fno-sys=harfbuzz \
        -fno-sys=fontconfig \
        -fno-sys=libpng \
        -fno-sys=zlib \
        -fno-sys=oniguruma \
        -fno-sys=libxml2 \
        "$@"
    fi
    exec ${zig_0_16}/bin/zig "$@"
  '';
  runtimeLibraries = [
    fontconfig
    freetype
    libGL
    libx11
    libxcb
    libxkbcommon
    libxml2
    wayland
    vulkan-loader
    llvmPackages_21.libcxx
  ];
in
assert lib.assertMsg (
  ghostty.version == "0.3.0"
) "Refresh packaging/ghostty-zig-deps.json when upgrading gpui-libghostty";
rustPlatform.buildRustPackage {
  pname = "farcaster";
  inherit (manifest.package) version;
  src = lib.cleanSourceWith {
    src = ../.;
    filter =
      path: type:
      lib.cleanSourceFilter path type
      && !(builtins.elem (builtins.baseNameOf path) [
        "target"
        ".direnv"
        ".git"
      ]);
  };
  cargoLock = {
    lockFile = ../Cargo.lock;
    # All git dependencies have full commit IDs in Cargo.lock.
    allowBuiltinFetchGit = true;
  };
  # Zig comes from ZIG below; a direct input would replace Cargo's build phases.
  nativeBuildInputs = [
    pkg-config
    cmake
    makeWrapper
    rustPlatform.bindgenHook
  ];
  buildInputs = runtimeLibraries;
  strictDeps = true;
  cargoBuildFlags = [
    "--bin"
    "farcaster"
  ];
  # The app's tests require a desktop. checks.<system>.startup tests the package.
  doCheck = false;
  preBuild = ''
    export ZIG="${ghosttyZig}"
    export GHOSTTY_NATIVE_CACHE_DIR="$NIX_BUILD_TOP/ghostty-native"
    export GHOSTTY_ZIG_PACKAGE_CACHE_DIR="$NIX_BUILD_TOP/ghostty-packages"
    export GHOSTTY_ZIG_GLOBAL_CACHE_DIR="$NIX_BUILD_TOP/ghostty-cache"
    export GHOSTTY_ZIG_SYSTEM_PACKAGE_DIR="${zigDeps}"
  '';
  postInstall = ''
    # cargoInstallHook already installed the executable.
    mv "$out/bin/farcaster" "$NIX_BUILD_TOP/farcaster-installed"
    sh scripts/install-linux.sh "$NIX_BUILD_TOP/farcaster-installed" "$out"
    wrapProgram "$out/bin/farcaster" \
      --set-default SSL_CERT_FILE "${cacert}/etc/ssl/certs/ca-bundle.crt" \
      --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath runtimeLibraries}" \
      --suffix PATH : "${
        lib.makeBinPath [
          git
          neovim
        ]
      }"
  '';
  meta = {
    inherit (manifest.package) description;
    homepage = "https://github.com/behzade/farcaster";
    license = lib.licenses.gpl3Plus;
    platforms = lib.platforms.linux;
    mainProgram = "farcaster";
  };
}
