{
  buildNpmPackage,
  lib,
}:

buildNpmPackage {
  pname = "pi-openai-server-compaction";
  version = "0.2.0";

  src = ../extensions/openai-server-compaction;

  npmDepsHash = "sha256-WzzsrKRTWjo5r0S+ZtIS9IvFA6MHg+CVSsahElvwjP4=";
  dontNpmBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -R src package.json package-lock.json node_modules *.md $out/
    runHook postInstall
  '';

  meta = {
    description = "OpenAI server-side compaction extension for Pi";
    homepage = "https://github.com/behzade/pi";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
