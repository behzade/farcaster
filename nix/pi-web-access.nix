{
  buildNpmPackage,
  fetchurl,
  lib,
}:

buildNpmPackage {
  pname = "pi-web-access";
  version = "0.15.0";

  src = fetchurl {
    url = "https://registry.npmjs.org/pi-web-access/-/pi-web-access-0.15.0.tgz";
    hash = "sha256-d7gpWnXZFCz7okWsuD1HCfdk6UUm9bawYyDBUMCIFXA=";
  };

  postPatch = ''
    cp ${./pi-web-access-package-lock.json} package-lock.json
  '';

  npmDepsHash = "sha256-F9/bnvVFc55RZv+NBkWAhV94Do4E4EHzEu5oOe5eaHw=";
  npmInstallFlags = [
    "--legacy-peer-deps"
    "--omit=dev"
    "--omit=peer"
  ];
  dontNpmBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    cp -R \
      *.md \
      *.mp4 \
      *.png \
      *.ts \
      node_modules \
      package-lock.json \
      package.json \
      "$out/"
    runHook postInstall
  '';

  meta = {
    description = "Web search and content extraction extension for Pi";
    homepage = "https://github.com/nicobailon/pi-web-access";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
