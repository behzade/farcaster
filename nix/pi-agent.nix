{
  coreExtensions,
  denseTools,
  openaiServerCompaction,
  piTerminal,
  projectTools,
  sandbox,
  sessionAgents,
  stdenvNoCC,
}:

stdenvNoCC.mkDerivation {
  pname = "pi-agent";
  version = "1.0.0";

  dontUnpack = true;

  installPhase = ''
    runHook preInstall

    mkdir -p "$out/extensions" "$out/skills" "$out/themes"

    substitute ${../SYSTEM.md} "$out/SYSTEM.md" \
      --replace-fail "@piCodingAgent@" "${piTerminal}/lib/pi-terminal/node_modules/@earendil-works/pi-coding-agent"
    ln -s ${../APPEND_SYSTEM.md} "$out/APPEND_SYSTEM.md"

    ln -s ${denseTools} "$out/extensions/dense-tools"
    ln -s ${openaiServerCompaction} "$out/extensions/openai-server-compaction"
    ln -s ${projectTools} "$out/extensions/project-tools"
    ln -s ${sandbox} "$out/extensions/sandbox"
    ln -s ${sessionAgents} "$out/extensions/session-agents"
    ln -s ${coreExtensions}/lib "$out/extensions/lib"
    ln -s ${coreExtensions}/node_modules "$out/extensions/node_modules"
    ln -s ${coreExtensions}/notifications.ts "$out/extensions/notifications.ts"
    ln -s ${coreExtensions}/codex-web-search.ts "$out/extensions/codex-web-search.ts"
    ln -s ${coreExtensions}/codex-web-search-core.ts "$out/extensions/codex-web-search-core.ts"
    ln -s ${coreExtensions}/prompt-inspector.ts "$out/extensions/prompt-inspector.ts"
    ln -s ${coreExtensions}/session-hooks.ts "$out/extensions/session-hooks.ts"
    ln -s ${coreExtensions}/title-state.ts "$out/extensions/title-state.ts"
    ln -s ${coreExtensions}/user-input.ts "$out/extensions/user-input.ts"
    ln -s ${coreExtensions}/user-invocations.ts "$out/extensions/user-invocations.ts"

    ln -s ${../prompts} "$out/prompts"
    ln -s ${../themes/gruvbox-dark-hard.json} "$out/themes/gruvbox-dark-hard.json"

    for skill in ${../skills}/*; do
      name="$(basename "$skill")"
      if [ -e "$out/skills/$name" ]; then
        echo "duplicate Pi skill: $name" >&2
        exit 1
      fi
      ln -s "$skill" "$out/skills/$name"
    done

    runHook postInstall
  '';
}
