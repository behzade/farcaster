{
  fetchFromGitHub,
  fetchPnpmDeps,
  lib,
  nodejs,
  pnpm_11,
  pnpmConfigHook,
  stdenvNoCC,
}:

stdenvNoCC.mkDerivation (finalAttrs: {
  pname = "pi-permission-system";
  version = "25.1.0";

  src = fetchFromGitHub {
    owner = "gotgenes";
    repo = "pi-packages";
    rev = "pi-permission-system-v25.1.0";
    hash = "sha256-qIjW6OWJq/Tb6gOYlWMGrSSnVrSOtFsYXBuXmZWDkog=";
  };

  patches = [
    ../patches/pi-permission-system-approval-transport.patch
  ];

  postPatch = ''
    legacy_name="pi-sub""agents"
    legacy_symbol="subscribeSubagent""Lifecycle"
    legacy_module="subagent-lifecycle-""events"
    legacy_channel="subagents:child"
    find \
      packages/pi-permission-system/src \
      packages/pi-permission-system/docs \
      -type f -exec sed -i \
        -e "/$legacy_name/d" \
        -e "/$legacy_symbol/d" \
        -e "/$legacy_module/d" \
        -e "/$legacy_channel/d" \
        {} +
    sed -i "/$legacy_name/d" packages/pi-permission-system/README.md
    rm packages/pi-permission-system/src/authority/subagent-lifecycle-events.ts
  '';

  pnpmDeps = fetchPnpmDeps {
    inherit (finalAttrs) pname version src;
    pnpm = pnpm_11;
    fetcherVersion = 3;
    pnpmWorkspaces = [ "@gotgenes/pi-permission-system" ];
    pnpmInstallFlags = [ "--prod" ];
    hash = "sha256-vUfJbPFkgKVky93k5silPN2bxFdGykguRR3r9n6q9lw=";
  };

  nativeBuildInputs = [
    nodejs
    pnpm_11
    pnpmConfigHook
  ];
  pnpmInstallFlags = [ "--prod" ];
  pnpmWorkspaces = [ "@gotgenes/pi-permission-system" ];

  dontBuild = true;

  installPhase = ''
    runHook preInstall
    pnpm \
      --filter @gotgenes/pi-permission-system \
      --offline \
      --config.inject-workspace-packages=true \
      deploy --prod "$out"
    runHook postInstall
  '';

  meta = {
    description = "Pi permission policy and parent-session approval transport";
    homepage = "https://github.com/gotgenes/pi-packages/tree/main/packages/pi-permission-system";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
})
