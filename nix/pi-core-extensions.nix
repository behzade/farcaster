{
  importNpmLock,
  lib,
  nodejs,
  stdenvNoCC,
}:

let
  files = lib.fileset.unions [
    ../extensions/package.json
    ../extensions/package-lock.json
    ../extensions/notifications.ts
    ../extensions/codex-web-search.ts
    ../extensions/codex-web-search-core.ts
    ../extensions/prompt-inspector.ts
    ../extensions/session-hooks.ts
    ../extensions/title-state.ts
    ../extensions/user-input.ts
    ../extensions/user-invocations.ts
    ../extensions/lib
  ];
  source = lib.fileset.toSource {
    root = ../extensions;
    fileset = files;
  };
  nodeModules = importNpmLock.buildNodeModules {
    npmRoot = source;
    inherit nodejs;
  };
in
stdenvNoCC.mkDerivation {
  pname = "pi-core-extensions";
  version = "1.0.0";

  src = source;

  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    cp -R *.ts lib package.json package-lock.json "$out/"
    cp -R ${nodeModules}/node_modules "$out/"
    runHook postInstall
  '';

  meta = {
    description = "Core Pi lifecycle and host-integration extensions";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
