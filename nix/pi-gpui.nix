{
  coreutils,
  craneLib,
  lib,
  makeWrapper,
  neovim,
  piTerminal,
  pkg-config,
  rustPlatform,
  sqlite,
  stdenv,
  alsa-lib,
  at-spi2-atk,
  cairo,
  cups,
  dbus,
  expat,
  fontconfig,
  freetype,
  glib,
  gtk3,
  libdrm,
  libgbm,
  libGL,
  libx11,
  libxcb,
  libxcomposite,
  libxdamage,
  libxext,
  libxfixes,
  libxkbcommon,
  libxkbfile,
  libxrandr,
  libxshmfence,
  nspr,
  nss,
  pango,
  pciutils,
  systemd,
  util-linux,
  vulkan-loader,
  wayland,
  zig_0_16,
}:

let
  pname = "pi-gpui";
  version = "0.1.0";
  sourceRoot = ../apps/pi-gpui;
  linuxRuntimeLibraryPath = lib.makeLibraryPath [
    libGL
    vulkan-loader
    wayland
  ];
  src = lib.cleanSourceWith {
    src = sourceRoot;
    filter = path: type:
      lib.cleanSourceFilter path type
      && !(type == "directory" && builtins.baseNameOf path == "target");
  };
  vendoredPathSources = lib.cleanSourceWith {
    src = sourceRoot;
    filter = path: _type:
      let
        relative = lib.removePrefix "${toString sourceRoot}/" (toString path);
      in
      relative == "third_party" || lib.hasPrefix "third_party/" relative;
  };
  cargoOutputHashes = {
    "git+https://github.com/proptest-rs/proptest?rev=3dca198a8fef1b32e3a66f1e1897c955b4dc5b5b#3dca198a8fef1b32e3a66f1e1897c955b4dc5b5b" =
      "sha256-p5NTcHhruI8QQvANACg8AMRVNmuvGxs2NLit+/8PaWo=";
    "git+https://github.com/longbridge/gpui-component?rev=bd833291311289f3468479d31b629d3de279d3d4#bd833291311289f3468479d31b629d3de279d3d4" =
      "sha256-5ZUdqetzhirAFdIr4oZLzovndZNDcbNc4arYAHZ0kRM=";
    "git+https://github.com/zed-industries/font-kit?rev=94b0f28166665e8fd2f53ff6d268a14955c82269#94b0f28166665e8fd2f53ff6d268a14955c82269" =
      "sha256-KXygi0olNQi5yM8eaJVykNDtbPMDjT+cWPBF8UrtXR4=";
    "git+https://github.com/zed-industries/reqwest.git?rev=c15662463bda39148ba154100dd44d3fba5873a4#c15662463bda39148ba154100dd44d3fba5873a4" =
      "sha256-p4SiUrOrbTlk/3bBrzN/mq/t+1Gzy2ot4nso6w6S+F8=";
    "git+https://github.com/zed-industries/scap?rev=4afea48c3b002197176fb19cd0f9b180dd36eaac#4afea48c3b002197176fb19cd0f9b180dd36eaac" =
      "sha256-BihiQHlal/eRsktyf0GI3aSWsUCW7WcICMsC2Xvb7kw=";
    "git+https://github.com/zed-industries/wasm_thread?rev=0cf96c7708dfb97ccf3da50347e25edcf75d6937#0cf96c7708dfb97ccf3da50347e25edcf75d6937" =
      "sha256-+lRLCIk0S6Y5ORYjDKsYYHia2FtoSoh+rWkQh7mnPBE=";
    "git+https://github.com/zed-industries/xim-rs.git?rev=16f35a2c881b815a2b6cdfd6687988e84f8447d8#16f35a2c881b815a2b6cdfd6687988e84f8447d8" =
      "sha256-pRT4Sz1JU9ros47/7pmIW9kosWOGMOItcnNd+VrvnpE=";
  };
  dummySrc = craneLib.mkDummySrc {
    inherit src;
    cargoLock = ../apps/pi-gpui/Cargo.lock;
    extraDummyScript = ''
      rm -rf "$out/third_party"
      cp -r ${vendoredPathSources}/third_party "$out/third_party"
    '';
  };

  commonArgs = {
    inherit pname src version;
    cargoLock = ../apps/pi-gpui/Cargo.lock;
    outputHashes = cargoOutputHashes;
    LIBSQLITE3_SYS_USE_PKG_CONFIG = "1";
    strictDeps = true;

    nativeBuildInputs = [
      makeWrapper
      pkg-config
      rustPlatform.bindgenHook
      zig_0_16
    ] ++ lib.optionals stdenv.hostPlatform.isLinux [
      util-linux
    ];

    buildInputs = [ sqlite ] ++ lib.optionals stdenv.hostPlatform.isLinux [
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
      systemd
      vulkan-loader
      wayland
    ];
  };

  cargoArtifacts = craneLib.buildDepsOnly (
    commonArgs
    // {
      inherit dummySrc;
    }
    # gpui-libghostty's Zig build fetches content-addressed Ghostty packages.
    # Pin the complete ARM dependency artifact so that this one fixed-output
    # build may fetch them while the final pi-gpui build remains offline.
    // lib.optionalAttrs (stdenv.hostPlatform.system == "aarch64-linux") {
      outputHashAlgo = "sha256";
      outputHashMode = "recursive";
      outputHash = "sha256-XFGkb4ejEE1JMfI2hsBMzmr7mLmB1o/fNUTw9DC+N2o=";
    }
  );
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;
    passthru = { inherit cargoArtifacts; };

    postInstall = ''
      ln -s "$out/bin/pi-gpui" "$out/bin/pi-gui"
      mkdir -p "$out/lib/pi-gpui"
      cp -R ${../apps/pi-gpui/extensions/companion} "$out/lib/pi-gpui/companion"
      chmod -R u+w "$out/lib/pi-gpui/companion"
    ''
    + lib.optionalString stdenv.hostPlatform.isDarwin ''
      app="$out/Applications/Pi.app"
      mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
      substitute ${../packaging/macos/Info.plist} "$app/Contents/Info.plist" \
        --replace-fail '@pi_path@' '${piTerminal}/bin/pi' \
        --replace-fail '@companion_extension@' "$out/lib/pi-gpui/companion/index.ts"
      install -Dm644 ${../packaging/macos/Pi.icns} "$app/Contents/Resources/Pi.icns"
      install -Dm755 "$out/bin/pi-gpui" "$app/Contents/MacOS/pi-gpui"
    '';

    postFixup = ''
      wrapProgram "$out/bin/pi-gpui" \
        --set PI_GUI_PI_PATH ${piTerminal}/bin/pi \
        --set PI_GUI_COMPANION_EXTENSION "$out/lib/pi-gpui/companion/index.ts" \
        --prefix PATH : ${lib.makeBinPath [ neovim ]} \
        ${lib.optionalString stdenv.hostPlatform.isLinux "--prefix PATH : ${lib.makeBinPath [ coreutils util-linux ]} --prefix LD_LIBRARY_PATH : ${linuxRuntimeLibraryPath}"}
    ''
    + lib.optionalString stdenv.hostPlatform.isDarwin ''
      # UNUserNotificationCenter rejects unsigned application bundles.
      /usr/bin/codesign --force --sign - "$out/Applications/Pi.app"
    '';

    meta = {
      description = "Native GPUI client for the Pi coding agent";
      license = lib.licenses.gpl3Plus;
      mainProgram = "pi-gui";
      platforms = lib.platforms.darwin ++ [
        "x86_64-linux"
        "aarch64-linux"
      ];
    };
  }
)
