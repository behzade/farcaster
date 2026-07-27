{ lib, stdenvNoCC }:

stdenvNoCC.mkDerivation {
  pname = "pi-sandbox-extension";
  version = "2.4.0";

  src = ../extensions/sandbox;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp background-jobs.ts index.ts codex-command.ts io-permissions.ts io-policy.ts sandbox-failures.ts package.json $out/
    install -Dm755 background-job.sh $out/background-job.sh
    runHook postInstall
  '';

  meta = {
    description = "Pi bash sandbox adapter using the installed Codex CLI";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
