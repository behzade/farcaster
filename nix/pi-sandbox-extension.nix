{
  buildNpmPackage,
  lib,
}:

buildNpmPackage {
  pname = "pi-sandbox-extension";
  version = "1.11.1";

  src = ../extensions/sandbox;
  npmDepsHash = "sha256-ueme3X4zugl469/oQBrJri/6WhyK5xL82UAl/7QZEvI=";

  dontNpmBuild = true;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp index.ts package.json package-lock.json $out/
    cp -R node_modules $out/
    patch -d "$out/node_modules/@anthropic-ai/sandbox-runtime" -p1 \
      < ${./patches/sandbox-runtime-no-host-mountpoints.patch}
    runHook postInstall
  '';

  meta = {
    description = "Pi sandbox extension using sandbox-exec on macOS and Bubblewrap on Linux";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
