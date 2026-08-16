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
  src = lib.cleanSource ../apps/pi-gpui;

  commonArgs = {
    inherit pname src version;
    cargoLock = ../apps/pi-gpui/Cargo.lock;
    outputHashes = {
      "git+https://github.com/longbridge/gpui-component?rev=41bee9c280b6708d3671b5e9b137a78f49394568#41bee9c280b6708d3671b5e9b137a78f49394568" =
        "sha256-NCnWBTvRMXB/2TLv70gaTwj0+666K1PBbDyABRQ/pwA=";
      "git+https://github.com/longbridge/gpui-component?rev=bc174a7ec4534b2a4174fddde314b38d30d69093#bc174a7ec4534b2a4174fddde314b38d30d69093" =
        "sha256-yum6KO/dD7lWUKuxBcCyNWDQvg5im74VHMK4mioKo+w=";
      "git+https://github.com/zed-industries/font-kit?rev=94b0f28166665e8fd2f53ff6d268a14955c82269#94b0f28166665e8fd2f53ff6d268a14955c82269" =
        "sha256-KXygi0olNQi5yM8eaJVykNDtbPMDjT+cWPBF8UrtXR4=";
      "git+https://github.com/zed-industries/reqwest.git?rev=c15662463bda39148ba154100dd44d3fba5873a4#c15662463bda39148ba154100dd44d3fba5873a4" =
        "sha256-p4SiUrOrbTlk/3bBrzN/mq/t+1Gzy2ot4nso6w6S+F8=";
      "git+https://github.com/zed-industries/scap?rev=4afea48c3b002197176fb19cd0f9b180dd36eaac#4afea48c3b002197176fb19cd0f9b180dd36eaac" =
        "sha256-BihiQHlal/eRsktyf0GI3aSWsUCW7WcICMsC2Xvb7kw=";
      "git+https://github.com/zed-industries/xim-rs.git?rev=16f35a2c881b815a2b6cdfd6687988e84f8447d8#16f35a2c881b815a2b6cdfd6687988e84f8447d8" =
        "sha256-pRT4Sz1JU9ros47/7pmIW9kosWOGMOItcnNd+VrvnpE=";
      "git+https://github.com/zed-industries/zed#90b15493109a2e1267cd3a6bc4c24cc0106ad5dc" =
        "sha256-ExK9u6S/fXBPOFgEaCjDsCXpL9c8Ey4RhYaIEANLvaQ=";
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

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
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
