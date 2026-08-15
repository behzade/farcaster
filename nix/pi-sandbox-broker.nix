{
  bubblewrap,
  lib,
  rustPlatform,
  stdenv,
}:

rustPlatform.buildRustPackage {
  pname = "pi-sandbox-broker";
  version = "0.4.0";

  src = lib.cleanSource ../sandbox-broker;
  cargoLock.lockFile = ../sandbox-broker/Cargo.lock;

  PI_BWRAP_PATH = lib.optionalString stdenv.hostPlatform.isLinux (lib.getExe bubblewrap);
  PI_CONCEAL_LAUNCHER_PATH = lib.optionalString stdenv.hostPlatform.isDarwin "${placeholder "out"}/libexec/pi-sandbox-conceal-launcher";
  PI_CONCEAL_SHIM_PATH = lib.optionalString stdenv.hostPlatform.isDarwin "${placeholder "out"}/lib/libpi-sandbox-conceal.dylib";

  postInstall = ''
    ${lib.optionalString stdenv.hostPlatform.isDarwin ''
      mkdir -p $out/lib $out/libexec
      $CC -std=c11 -Os -Wall -Wextra -Werror \
        -o $out/libexec/pi-sandbox-conceal-launcher \
        native/macos-conceal-launcher.c
      $CC -std=c11 -Os -Wall -Wextra -Werror -dynamiclib \
        -o $out/lib/libpi-sandbox-conceal.dylib \
        native/macos-conceal-shim.c
    ''}
    install -Dm644 LICENSE-APACHE $out/share/doc/pi-sandbox-broker/LICENSE-APACHE
    install -Dm644 NOTICE $out/share/doc/pi-sandbox-broker/NOTICE
    install -Dm644 UPSTREAM.md $out/share/doc/pi-sandbox-broker/UPSTREAM.md
    install -Dm644 PROTOCOL.md $out/share/doc/pi-sandbox-broker/PROTOCOL.md
    install -Dm644 THREAT_MODEL.md $out/share/doc/pi-sandbox-broker/THREAT_MODEL.md
  '';

  meta = {
    description = "Pi native sandbox broker with macOS Seatbelt and Linux Bubblewrap backends";
    license = with lib.licenses; [
      asl20
      mit
    ];
    platforms = lib.platforms.darwin ++ [
      "x86_64-linux"
      "aarch64-linux"
    ];
    mainProgram = "pi-sandbox-broker";
  };
}
