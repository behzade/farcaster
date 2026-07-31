{
  bun,
  cacert,
  lib,
  makeWrapper,
  stdenvNoCC,
  dependencySystem ? stdenvNoCC.hostPlatform.system,
}:

let
  dependencyTarget = {
    aarch64-darwin = {
      cpu = "arm64";
      os = "darwin";
      hash = "sha256-Qs+DINzaRz87YPxpCUzhBEpS0xtDjMyd77gQYBhpcvg=";
    };
    x86_64-linux = {
      cpu = "x64";
      os = "linux";
      hash = "sha256-vwbO0zmGiPyJplXg7TnPL8lJOknGmuiGuIEOsrpWBOQ=";
    };
  }.${dependencySystem};

  source = lib.fileset.toSource {
    root = ../apps/pi-opentui;
    fileset = lib.fileset.unions [
      ../apps/pi-opentui/bun.lock
      ../apps/pi-opentui/package.json
      ../apps/pi-opentui/patches
      ../apps/pi-opentui/src
    ];
  };

  bunDeps = stdenvNoCC.mkDerivation {
    pname = "pi-opentui-bun-deps";
    version = "0.0.1";
    src = source;

    nativeBuildInputs = [
      bun
      cacert
    ];

    dontConfigure = true;
    dontFixup = true;

    buildPhase = ''
      runHook preBuild
      export HOME="$TMPDIR/home"
      mkdir -p "$HOME"
      bun install --frozen-lockfile --ignore-scripts \
        --cpu=${dependencyTarget.cpu} \
        --os=${dependencyTarget.os}
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      cp -R node_modules "$out"
      runHook postInstall
    '';

    outputHashAlgo = "sha256";
    outputHashMode = "recursive";
    outputHash = dependencyTarget.hash;
  };
in
stdenvNoCC.mkDerivation {
  pname = "pi-opentui";
  version = "0.0.1";
  src = source;

  nativeBuildInputs = [ makeWrapper ];
  dontBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p "$out/bin" "$out/lib/pi-opentui"
    cp -R src package.json "$out/lib/pi-opentui/"
    cp -R ${bunDeps} "$out/lib/pi-opentui/node_modules"
    makeWrapper ${bun}/bin/bun "$out/bin/pi" \
      --add-flags "$out/lib/pi-opentui/src/main.ts"
    runHook postInstall
  '';

  meta = {
    description = "OpenTUI front end for the Pi coding agent";
    homepage = "https://github.com/behzade/pi";
    license = lib.licenses.mit;
    mainProgram = "pi";
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
