{ lib, stdenvNoCC }:

stdenvNoCC.mkDerivation {
  pname = "pi-sandbox-extension";
  version = "2.7.0";

  src = ../extensions/sandbox;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp background-jobs.ts broker-client.ts broker-policy.ts index.ts codex-command.ts declared-permissions.ts io-permissions.ts io-policy.ts sandbox-failures.ts package.json $out/
    substituteInPlace $out/index.ts \
      --replace-fail '@PI_SANDBOX_BROKER@' '/unreleased/pi-sandbox-broker'
    install -Dm755 background-job.sh $out/background-job.sh
    runHook postInstall
  '';

  meta = {
    description = "Pi bash sandbox adapter with Codex and an unreleased native client";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
