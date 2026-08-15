{
  buildNpmPackage,
  fetchFromGitHub,
  lib,
}:

buildNpmPackage {
  pname = "pi-subagents";
  version = "0.49.0";

  src = fetchFromGitHub {
    owner = "nicobailon";
    repo = "pi-subagents";
    rev = "v0.49.0";
    hash = "sha256-qcapXaNEbQHwjPfIt3wcjHdeLiZC/ReFj2BtvwWkHFc=";
  };

  patches = [
    ../patches/pi-subagents-local-hardening.patch
  ];

  npmDepsHash = "sha256-VeUptKmEiwuMyhAozpoIx8SACsJMjk7EFNcE8EG8lhU=";
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
