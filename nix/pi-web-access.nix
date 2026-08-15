{
  buildNpmPackage,
  fetchFromGitHub,
  lib,
}:

buildNpmPackage {
  pname = "pi-web-access";
  version = "0.23.0";

  src = fetchFromGitHub {
    owner = "nicobailon";
    repo = "pi-web-access";
    rev = "v0.23.0";
    hash = "sha256-q/TZUkgeC/W/Ft7RMVIDc6m/Dsj2amicHhSeCbzk05E=";
  };

  patches = [ ../patches/pi-web-access-default-openai.patch ];

  postPatch = ''
    cp ${./pi-web-access-package-lock.json} package-lock.json
  '';

  npmDepsHash = "sha256-TUOiefsKK1rbjn2PGUfuanLHg39bm08NvMOZTVMqfLo=";
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
