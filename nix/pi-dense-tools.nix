{
  fetchurl,
  git,
  lib,
  makeWrapper,
  nodejs,
  stdenvNoCC,
}:

let
  pierreDiffs = fetchurl {
    url = "https://esm.sh/@pierre/diffs@1.3.0-rc.1/es2022/diffs.bundle.mjs";
    hash = "sha256-hAR/hp8BordM+efFthA3dvG5Z4JpJJb2GCP8EXjCjC0=";
  };
in
stdenvNoCC.mkDerivation {
  pname = "pi-dense-tools-extension";
  version = "0.4.0";

  src = ../extensions/dense-tools;

  nativeBuildInputs = [ makeWrapper ];

  installPhase = ''
    runHook preInstall
    mkdir -p "$out/themes"
    cp index.ts pierre-edit.ts pierre-renderer.ts terminal-text.ts pi-diff.ts package.json NOTICE.md "$out/"
    cp ${../themes/gruvbox-dark-hard.json} $out/themes/gruvbox-dark-hard.json
    cp ${pierreDiffs} $out/diffs.bundle.mjs
    substituteInPlace $out/diffs.bundle.mjs \
      --replace-fail '"/node/process.mjs"' '"node:process"' \
      --replace-fail '"/node/buffer.mjs"' '"node:buffer"'
    substituteInPlace $out/pi-diff.ts \
      --replace-fail '@PI_DIFF_GIT@' '${lib.getExe git}'
    makeWrapper ${lib.getExe nodejs} "$out/bin/pi-diff" \
      --add-flags "--experimental-strip-types" \
      --add-flags "$out/pi-diff.ts"
    runHook postInstall
  '';

  meta = {
    description = "Dense Pi tool renderers and the pi-diff terminal formatter";
    license = lib.licenses.mit;
    mainProgram = "pi-diff";
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
