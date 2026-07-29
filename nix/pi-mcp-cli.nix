{
  autoPatchelfHook,
  fetchurl,
  lib,
  makeWrapper,
  stdenv,
  stdenvNoCC,
}:

let
  version = "0.3.0";
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
  artifact =
    artifacts.${stdenvNoCC.hostPlatform.system}
      or (throw "mcp-cli is not packaged for ${stdenvNoCC.hostPlatform.system}");
  binary = fetchurl {
    url = "https://github.com/philschmid/mcp-cli/releases/download/v${version}/${artifact.name}";
    inherit (artifact) hash;
  };
in
stdenvNoCC.mkDerivation {
  pname = "mcp-cli";
  inherit version;

  dontUnpack = true;
  nativeBuildInputs = [
    makeWrapper
  ] ++ lib.optionals stdenvNoCC.hostPlatform.isLinux [
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
}
