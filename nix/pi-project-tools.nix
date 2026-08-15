{
  importNpmLock,
  lib,
  nodejs,
  stdenvNoCC,
}:

let
  source = ../extensions/project-tools;
  nodeModules = importNpmLock.buildNodeModules {
    npmRoot = source;
    inherit nodejs;
  };
in
stdenvNoCC.mkDerivation {
  pname = "pi-project-tools";
  version = "0.1.0";

  src = source;

  installPhase = ''
    runHook preInstall
    mkdir -p "$out"
    cp -R README.md package.json package-lock.json src "$out/"
    cp -R ${nodeModules}/node_modules "$out/"
    runHook postInstall
  '';

  meta = {
    description = "Trusted project-scoped Effect tools for Pi";
    homepage = "https://github.com/behzade/pi";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
