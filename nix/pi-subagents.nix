{
  buildNpmPackage,
  fetchFromGitHub,
  lib,
}:

buildNpmPackage {
  pname = "pi-subagents";
  version = "0.36.0";

  src = fetchFromGitHub {
    owner = "nicobailon";
    repo = "pi-subagents";
    rev = "v0.36.0";
    hash = "sha256-o4x//aNXBweHPiwUhGp08iVZ/LeLjkFSzEm60G8EzxY=";
  };

  patches = [
    ../patches/pi-subagents-quiet-optional-result-intercom.patch
    ../patches/pi-subagents-wait-steering.patch
    ../patches/pi-subagents-orchestration-hardening.patch
  ];

  npmDepsHash = "sha256-Si1Fc01ORGdauY+6+Us3eRLuFzmZaAaAicZTQdcUFHY=";
  npmInstallFlags = [ "--omit=dev" ];
  dontNpmBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    cp -R \
      agents \
      prompts \
      skills \
      src \
      CHANGELOG.md \
      README.md \
      index.ts \
      node_modules \
      package-lock.json \
      package.json \
      "$out/"
    install -Dm644 ${./pi-subagents-config.json} "$out/config.json"
    runHook postInstall
  '';

  meta = {
    description = "Async subagent delegation and supervision for Pi";
    homepage = "https://github.com/nicobailon/pi-subagents";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
