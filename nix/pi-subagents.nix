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

    mkdir -p "$out/agent-feedback/lib"
    install -Dm644 ${../extensions/agent-feedback.ts} "$out/agent-feedback/index.ts"
    install -Dm644 ${../extensions/lib/agent-feedback.ts} "$out/agent-feedback/lib/agent-feedback.ts"
    for agent in "$out"/agents/*.md; do
      toolLine="$(grep -m1 '^tools:' "$agent")"
      substituteInPlace "$agent" \
        --replace-fail "$toolLine" "$toolLine, report_pi_feedback
extensions:
subagentOnlyExtensions: $out/agent-feedback/index.ts"
    done

    runHook postInstall
  '';

  meta = {
    description = "Async subagent delegation and supervision for Pi";
    homepage = "https://github.com/nicobailon/pi-subagents";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin ++ lib.platforms.linux;
  };
}
