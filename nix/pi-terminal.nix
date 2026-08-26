{
  bun,
  cacert,
  lib,
  makeWrapper,
  mcpCli,
  nodejs_24,
  stdenvNoCC,
  dependencySystem ? stdenvNoCC.hostPlatform.system,
}:

let
  dependencyTarget = {
    aarch64-darwin = {
      cpu = "arm64";
      os = "darwin";
      hash = "sha256-JO86+OIg2/JaWL7TmYMnvfNgU8eSnLJwHZVIFHzyr0k=";
    };
    aarch64-linux = {
      cpu = "arm64";
      os = "linux";
      hash = "sha256-Z8SVP6SWJlUVus3cys0mdk2HzOpDCzmfoKdS/zntdy8=";
    };
    x86_64-linux = {
      cpu = "x64";
      os = "linux";
      hash = "sha256-6OWCWEMg7RpgAKHJTQrqGa70iL2+2rIu/fZdjaSQ4MY=";
    };
  }.${dependencySystem};

  dependencyFiles = lib.fileset.unions [
    ../apps/pi-terminal/bun.lock
    ../apps/pi-terminal/package.json
    (../apps/pi-terminal/patches + "/@earendil-works%2Fpi-ai@0.84.2.patch")
  ];

  dependencySource = lib.fileset.toSource {
    root = ../apps/pi-terminal;
    fileset = dependencyFiles;
  };

  source = lib.fileset.toSource {
    root = ../apps/pi-terminal;
    fileset = lib.fileset.unions [
      dependencyFiles
      ../apps/pi-terminal/patches
      ../apps/pi-terminal/test
    ];
  };

  bunDeps = stdenvNoCC.mkDerivation {
    pname = "pi-terminal-bun-deps";
    version = "0.84.2";
    src = dependencySource;

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
  pname = "pi-terminal";
  version = "0.84.2";
  src = source;

  nativeBuildInputs = [
    bun
    makeWrapper
  ];

  dontConfigure = true;

  buildPhase = ''
    runHook preBuild
    cp -R ${bunDeps} node_modules
    chmod -R u+w node_modules/@earendil-works/pi-coding-agent
    patch -d node_modules/@earendil-works/pi-coding-agent -p1 \
      < patches/@earendil-works%2Fpi-coding-agent@0.84.2.patch
    bun test
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p "$out/bin" "$out/lib/pi-terminal"
    cp package.json "$out/lib/pi-terminal/"
    cp -R node_modules "$out/lib/pi-terminal/node_modules"
    makeWrapper ${nodejs_24}/bin/node "$out/bin/pi" \
      --add-flags "$out/lib/pi-terminal/node_modules/@earendil-works/pi-coding-agent/dist/cli.js" \
      --prefix PATH : ${mcpCli}/bin
    runHook postInstall
  '';

  meta = {
    description = "Pi coding agent terminal client with cached OpenAI compaction support";
    homepage = "https://github.com/earendil-works/pi";
    license = lib.licenses.mit;
    mainProgram = "pi";
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
