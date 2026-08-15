{
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

rustPlatform.buildRustPackage {
  pname = "pi-gpui";
  version = "0.1.0";

  src = lib.cleanSource ../apps/pi-gpui;

  cargoLock = {
    lockFile = ../apps/pi-gpui/Cargo.lock;
    outputHashes = {
      "collections-0.1.0" = "sha256-ExK9u6S/fXBPOFgEaCjDsCXpL9c8Ey4RhYaIEANLvaQ=";
      "gpui-component-0.5.2" = "sha256-yum6KO/dD7lWUKuxBcCyNWDQvg5im74VHMK4mioKo+w=";
      "gpui-fps-0.1.0" = "sha256-NCnWBTvRMXB/2TLv70gaTwj0+666K1PBbDyABRQ/pwA=";
      "xim-ctext-0.3.0" = "sha256-pRT4Sz1JU9ros47/7pmIW9kosWOGMOItcnNd+VrvnpE=";
      "zed-font-kit-0.14.1-zed" = "sha256-KXygi0olNQi5yM8eaJVykNDtbPMDjT+cWPBF8UrtXR4=";
      "zed-reqwest-0.12.15-zed" = "sha256-p4SiUrOrbTlk/3bBrzN/mq/t+1Gzy2ot4nso6w6S+F8=";
      "zed-scap-0.0.8-zed" = "sha256-BihiQHlal/eRsktyf0GI3aSWsUCW7WcICMsC2Xvb7kw=";
    };
  };

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

  postInstall = ''
    ln -s "$out/bin/pi-gpui" "$out/bin/pi-gui"
  ''
  + lib.optionalString stdenv.hostPlatform.isDarwin ''
    app="$out/Applications/Pi.app"
    mkdir -p "$app/Contents/MacOS"
    install -Dm644 ${../packaging/macos/Info.plist} "$app/Contents/Info.plist"
    substitute ${../packaging/macos/launch.sh} "$app/Contents/MacOS/pi-gpui" \
      --replace-fail '@binary@' "$out/bin/pi-gpui"
    chmod +x "$app/Contents/MacOS/pi-gpui"
  '';

  postFixup = ''
    wrapProgram "$out/bin/pi-gpui" \
      --prefix PATH : ${lib.makeBinPath [ piTerminal ]}
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
