{
  importNpmLock,
  lib,
  nodejs,
  stdenvNoCC,
}:

let
  source = ../extensions/subagents;
  nodeModules = importNpmLock.buildNodeModules {
    npmRoot = source;
    inherit nodejs;
  };
in
stdenvNoCC.mkDerivation {
  pname = "pi-session-agents-extension";
  version = "1.0.0";

  src = source;

  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    cp adapter.ts contract.ts core.ts index.ts package.json package-lock.json "$out/"
    cp -R ${nodeModules}/node_modules "$out/"
    runHook postInstall
  '';

  meta = {
    description = "Plain persistent child Pi sessions";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
