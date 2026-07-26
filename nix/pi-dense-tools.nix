{ fetchurl, stdenvNoCC }:

let
  pierreDiffs = fetchurl {
    url = "https://esm.sh/@pierre/diffs@1.3.0-rc.1/es2022/diffs.bundle.mjs";
    hash = "sha256-hAR/hp8BordM+efFthA3dvG5Z4JpJJb2GCP8EXjCjC0=";
  };
in
stdenvNoCC.mkDerivation {
  pname = "pi-dense-tools-extension";
  version = "0.1.0";

  src = ../extensions/dense-tools;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp index.ts pierre-edit.ts package.json NOTICE.md $out/
    cp ${pierreDiffs} $out/diffs.bundle.mjs
    substituteInPlace $out/diffs.bundle.mjs \
      --replace-fail '"/node/process.mjs"' '"node:process"' \
      --replace-fail '"/node/buffer.mjs"' '"node:buffer"'
    runHook postInstall
  '';
}
