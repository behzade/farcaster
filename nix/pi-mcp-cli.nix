{
  autoPatchelfHook,
  bun,
  cacert,
  fetchFromGitHub,
  fetchurl,
  lib,
  makeWrapper,
  stdenv,
  stdenvNoCC,
}:

let
  version = "0.3.0";
  system = stdenvNoCC.hostPlatform.system;
  artifacts = {
    aarch64-darwin = {
      name = "mcp-cli-darwin-arm64";
      hash = "sha256-vpkd8KEl4c+aAi/m/84jZgVSL9E4JDXOWP/fmrpmQvI=";
    };
    x86_64-linux = {
      name = "mcp-cli-linux-x64";
      hash = "sha256-dncvKQ7aqFbL7JZ9EsM8ub9Jz/AU9Vow0EJFz4lwgXw=";
    };
  };
  binaryPackage = artifact:
    let
      binary = fetchurl {
        url = "https://github.com/philschmid/mcp-cli/releases/download/v${version}/${artifact.name}";
        inherit (artifact) hash;
      };
    in
    stdenvNoCC.mkDerivation {
      pname = "mcp-cli";
      inherit version;

      dontUnpack = true;
      nativeBuildInputs = [ makeWrapper ] ++ lib.optionals stdenvNoCC.hostPlatform.isLinux [
        autoPatchelfHook
      ];
      buildInputs = lib.optionals stdenvNoCC.hostPlatform.isLinux [
        stdenv.cc.cc.lib
      ];

      installPhase = ''
        runHook preInstall
        install -Dm755 ${binary} "$out/libexec/mcp-cli"
        makeWrapper "$out/libexec/mcp-cli" "$out/bin/mcp-cli" \
          --set MCP_NO_DAEMON 1
        runHook postInstall
      '';

      meta = {
        description = "Stateless CLI for discovering and calling MCP tools";
        homepage = "https://github.com/philschmid/mcp-cli";
        license = lib.licenses.mit;
        mainProgram = "mcp-cli";
        platforms = builtins.attrNames artifacts;
      };
    };
  source = fetchFromGitHub {
    owner = "philschmid";
    repo = "mcp-cli";
    rev = "v${version}";
    hash = lib.fakeHash;
  };
  sourceDeps = stdenvNoCC.mkDerivation {
    pname = "mcp-cli-bun-deps";
    inherit version;
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
      bun install --frozen-lockfile --ignore-scripts --cpu=arm64 --os=linux
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      cp -R node_modules "$out"
      runHook postInstall
    '';

    outputHashAlgo = "sha256";
    outputHashMode = "recursive";
    outputHash = lib.fakeHash;
  };
  sourcePackage = stdenvNoCC.mkDerivation {
    pname = "mcp-cli";
    inherit version;
    src = source;

    nativeBuildInputs = [
      autoPatchelfHook
      bun
      makeWrapper
    ];
    buildInputs = [ stdenv.cc.cc.lib ];

    buildPhase = ''
      runHook preBuild
      cp -R ${sourceDeps} node_modules
      bun build --compile --minify --target=bun-linux-arm64 \
        src/index.ts --outfile dist/mcp-cli-linux-arm64
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      install -Dm755 dist/mcp-cli-linux-arm64 "$out/libexec/mcp-cli"
      makeWrapper "$out/libexec/mcp-cli" "$out/bin/mcp-cli" \
        --set MCP_NO_DAEMON 1
      runHook postInstall
    '';

    meta = {
      description = "Stateless CLI for discovering and calling MCP tools";
      homepage = "https://github.com/philschmid/mcp-cli";
      license = lib.licenses.mit;
      mainProgram = "mcp-cli";
      platforms = [ "aarch64-linux" ];
    };
  };
in
if builtins.hasAttr system artifacts then
  binaryPackage artifacts.${system}
else if system == "aarch64-linux" then
  sourcePackage
else
  throw "mcp-cli is not packaged for ${system}"
