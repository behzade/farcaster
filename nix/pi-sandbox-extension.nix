{
  lib,
  sandboxBroker ? null,
  stdenvNoCC,
}:

let
  brokerRoot = if sandboxBroker == null then "/unreleased/pi-sandbox-broker" else sandboxBroker;
in
stdenvNoCC.mkDerivation {
  pname = "pi-sandbox-extension";
  version = "2.9.0";

  src = ../extensions/sandbox;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp background-jobs.ts broker-client.ts broker-policy.ts index.ts codex-command.ts declared-permissions.ts development-caches.ts io-permissions.ts io-policy.ts native-denials.ts native-sandbox-ops.ts sandbox-failures.ts package.json $out/
    substituteInPlace $out/index.ts \
      --replace-fail '@PI_SANDBOX_BROKER@' '${brokerRoot}'
    install -Dm755 background-job.sh $out/background-job.sh
    runHook postInstall
  '';

  meta = {
    description = "Pi bash sandbox adapter with Codex and native broker clients";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
