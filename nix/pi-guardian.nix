{
  buildNpmPackage,
  lib,
}:

buildNpmPackage {
  pname = "pi-guardian";
  version = "1.0.0";

  src = ../extensions/guardian;
  npmDepsHash = "sha256-iUQLx5r7ltQie+tuESbcXFsqvlyai5Hp0Y8qPKeSacs=";
  npmInstallFlags = [
    "--omit=dev"
    "--omit=peer"
  ];
  dontNpmBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    cp -R \
      LICENSE \
      README.md \
      UPSTREAM.md \
      node_modules \
      package-lock.json \
      package.json \
      preflight \
      "$out/"
    runHook postInstall
  '';

  meta = {
    description = "Vendored automatic permission reviewer for Pi";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
