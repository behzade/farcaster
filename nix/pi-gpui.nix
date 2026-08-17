{
  craneLib,
  direnv,
  lib,
  makeWrapper,
  piTerminal,
  pkg-config,
  rustPlatform,
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
  vulkan-loader,
  wayland,
}:

let
  pname = "pi-gpui";
  version = "0.1.0";
  sourceRoot = ../apps/pi-gpui;
  src = lib.cleanSourceWith {
    src = sourceRoot;
    filter = path: type:
      lib.cleanSourceFilter path type
      && !(type == "directory" && builtins.baseNameOf path == "target");
  };
  gpuiPathSources = lib.cleanSourceWith {
    src = sourceRoot;
    filter = path: _type:
      let
        relative = lib.removePrefix "${toString sourceRoot}/" (toString path);
      in
      relative == "third_party"
      || lib.hasPrefix "third_party/zed-gpui-cc053a4" relative;
  };
  dummySrc = craneLib.mkDummySrc {
    inherit src;
    cargoLock = ../apps/pi-gpui/Cargo.lock;
    extraDummyScript = ''
      rm -rf "$out/third_party"
      cp -r ${gpuiPathSources}/third_party "$out/third_party"
    '';
  };

  commonArgs = {
    inherit pname src version;
    cargoLock = ../apps/pi-gpui/Cargo.lock;
    outputHashes = {
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
    strictDeps = true;

    nativeBuildInputs = [
      makeWrapper
      pkg-config
      rustPlatform.bindgenHook
    ];

    buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
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
  );
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;

    postInstall = ''
      ln -s "$out/bin/pi-gpui" "$out/bin/pi-gui"
    ''
    + lib.optionalString stdenv.hostPlatform.isDarwin ''
      app="$out/Applications/Pi.app"
      mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
      install -Dm644 ${../packaging/macos/Info.plist} "$app/Contents/Info.plist"
      install -Dm644 ${../packaging/macos/Pi.icns} "$app/Contents/Resources/Pi.icns"
      substitute ${../packaging/macos/launch.sh} "$app/Contents/MacOS/pi-gpui" \
        --replace-fail '@binary@' "$out/bin/pi-gpui"
      chmod +x "$app/Contents/MacOS/pi-gpui"
    '';

    postFixup = ''
      wrapProgram "$out/bin/pi-gpui" \
        --set PI_GUI_PI_PATH ${piTerminal}/bin/pi \
        --prefix PATH : ${lib.makeBinPath [ direnv piTerminal ]}
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
