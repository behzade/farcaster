{
  importNpmLock,
  lib,
  mcpCli,
  nodejs,
  sandboxBroker ? null,
  stdenvNoCC,
}:

let
  brokerRoot = if sandboxBroker == null then "/unreleased/pi-sandbox-broker" else sandboxBroker;
  source = ../extensions/sandbox;
  nodeModules = importNpmLock.buildNodeModules {
    npmRoot = source;
    inherit nodejs;
  };
in
stdenvNoCC.mkDerivation {
  pname = "pi-sandbox-extension";
  version = "3.0.0";

  src = source;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp background-jobs.ts broker-client.ts broker-policy.ts denial-summary.ts index.ts native-background-jobs.ts native-network-proxy.ts network-policy.ts sandbox-config.ts development-caches.ts io-permissions.ts io-policy.ts native-sandbox-ops.ts permission-system-approval.ts project-policy-store.ts project-policy.ts tool-schemas.ts package.json package-lock.json $out/
    cp -R ${nodeModules}/node_modules "$out/"
    substituteInPlace $out/index.ts \
      --replace-fail '@PI_SANDBOX_BROKER@' '${brokerRoot}'
    substituteInPlace $out/sandbox-config.ts \
      --replace-fail '@PI_MCP_CLI@' '${mcpCli}/bin/mcp-cli'
    runHook postInstall
  '';

  meta = {
    description = "Pi bash sandbox adapter with the native sandbox broker";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
