{
  denseTools,
  openaiServerCompaction,
  permissionSystem,
  piTerminal,
  projectTools,
  sandbox,
  stdenvNoCC,
  subagents,
  webAccess,
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
    ln -s ${permissionSystem} "$out/extensions/permission-system"
    ln -s ${projectTools} "$out/extensions/project-tools"
    ln -s ${sandbox} "$out/extensions/sandbox"
    ln -s ${subagents} "$out/extensions/subagent"
    ln -s ${webAccess} "$out/extensions/web-access"
    ln -s ${../extensions/lib} "$out/extensions/lib"
    ln -s ${../extensions/agent-feedback.ts} "$out/extensions/agent-feedback.ts"
    ln -s ${../extensions/notifications.ts} "$out/extensions/notifications.ts"
    ln -s ${../extensions/prompt-inspector.ts} "$out/extensions/prompt-inspector.ts"
    ln -s ${../extensions/session-hooks.ts} "$out/extensions/session-hooks.ts"
    ln -s ${../extensions/title-state.ts} "$out/extensions/title-state.ts"
    ln -s ${../extensions/user-input.ts} "$out/extensions/user-input.ts"

    ln -s ${subagents}/prompts "$out/prompts"
    ln -s ${../themes/gruvbox-dark-hard.json} "$out/themes/gruvbox-dark-hard.json"

    for skill in ${subagents}/skills/* ${../skills}/*; do
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
