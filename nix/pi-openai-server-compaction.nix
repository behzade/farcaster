{
  buildNpmPackage,
  fetchFromGitHub,
  lib,
}:

buildNpmPackage {
  pname = "pi-openai-server-compaction";
  version = "0.1.0-unstable-2026-07-22";

  src = fetchFromGitHub {
    owner = "algal";
    repo = "pi-openai-server-compaction";
    rev = "c6d593087709e9481223dc6c6c2269b371b5e055";
    hash = "sha256-SFGcISdYblxGonhipIHPAOons8MdwYtu+A+WbHnNSVg=";
  };

  patches = [ ./patches/pi-openai-server-compaction-remote-only.patch ];

  postPatch = ''
    cp ${./pi-openai-server-compaction-package.json} package.json
    cp ${./pi-openai-server-compaction-package-lock.json} package-lock.json
  '';

  npmDepsHash = "sha256-JhHnpwZfo8YEQUO9Ip95gOVXulj9uAMLNbj4CnrEf0M=";
  dontNpmBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -R src package.json package-lock.json node_modules $out/
    runHook postInstall
  '';

  meta = {
    description = "OpenAI server-side compaction extension for Pi";
    homepage = "https://github.com/algal/pi-openai-server-compaction";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
