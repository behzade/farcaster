{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "pi-sandbox-broker";
  version = "0.3.0";

  src = lib.cleanSource ../sandbox-broker;
  cargoLock.lockFile = ../sandbox-broker/Cargo.lock;

  postInstall = ''
    install -Dm644 LICENSE-APACHE $out/share/doc/pi-sandbox-broker/LICENSE-APACHE
    install -Dm644 NOTICE $out/share/doc/pi-sandbox-broker/NOTICE
    install -Dm644 UPSTREAM.md $out/share/doc/pi-sandbox-broker/UPSTREAM.md
    install -Dm644 PROTOCOL.md $out/share/doc/pi-sandbox-broker/PROTOCOL.md
    install -Dm644 THREAT_MODEL.md $out/share/doc/pi-sandbox-broker/THREAT_MODEL.md
  '';

  meta = {
    description = "Pi native sandbox broker with a macOS Seatbelt backend";
    license = with lib.licenses; [
      asl20
      mit
    ];
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
    mainProgram = "pi-sandbox-broker";
  };
}
