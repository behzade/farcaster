{ lib, stdenvNoCC }:

stdenvNoCC.mkDerivation {
  pname = "pi-sandbox-extension";
  version = "2.1.0";

  src = ../extensions/sandbox;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp index.ts codex-command.ts io-permissions.ts io-policy.ts sandbox-denials.ts package.json $out/
    runHook postInstall
  '';

  meta = {
    description = "Pi bash sandbox adapter using the installed Codex CLI";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
