{
  buildNpmPackage,
  lib,
}:

buildNpmPackage {
  pname = "pi-mcp-adapter";
  version = "0.2.0";

  src = ../extensions/mcp-adapter;

  npmDepsHash = "sha256-edZSiHO0J8FDlsaKDF2tmtby0jFhXc7CgmzdZg8rqBo=";
  dontNpmBuild = true;
  doCheck = true;
  checkPhase = ''
    runHook preCheck
    npm run check
    runHook postCheck
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    cp approval-service.ts index.ts endpoint.ts package.json package-lock.json "$out/"
    cp -R node_modules "$out/"
    runHook postInstall
  '';

  meta = {
    description = "Lazy Streamable HTTP MCP adapter for Pi";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
