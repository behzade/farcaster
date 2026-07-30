{
  description = "Behzad's reviewed Pi coding-agent extensions";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs =
    {
      nixpkgs,
      self,
    }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: import nixpkgs { inherit system; };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          mcpCli = pkgs.callPackage ./nix/pi-mcp-cli.nix { };
          sandboxBroker = pkgs.callPackage ./nix/pi-sandbox-broker.nix { };
          sandbox = pkgs.callPackage ./nix/pi-sandbox-extension.nix {
            inherit mcpCli sandboxBroker;
          };
          denseTools = pkgs.callPackage ./nix/pi-dense-tools.nix { };
          subagents = pkgs.callPackage ./nix/pi-subagents.nix { };
          webAccess = pkgs.callPackage ./nix/pi-web-access.nix { };
          openaiServerCompaction = pkgs.callPackage ./nix/pi-openai-server-compaction.nix { };
          permissionSystem = pkgs.callPackage ./nix/pi-permission-system.nix { };
          agent = pkgs.callPackage ./nix/pi-agent.nix {
            inherit
              denseTools
              openaiServerCompaction
              permissionSystem
              sandbox
              subagents
              webAccess
              ;
          };
        in
        {
          inherit agent sandbox subagents;
          mcp-cli = mcpCli;
          permission-system = permissionSystem;
          sandbox-broker = sandboxBroker;
          dense-tools = denseTools;
          openai-server-compaction = openaiServerCompaction;
          web-access = webAccess;
          default = agent;
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          denseTools = pkgs.callPackage ./nix/pi-dense-tools.nix { };
          mcpCli = pkgs.callPackage ./nix/pi-mcp-cli.nix { };
          webAccess = pkgs.callPackage ./nix/pi-web-access.nix { };
        in
        {
          sandbox-tests = pkgs.runCommand "pi-sandbox-tests" { nativeBuildInputs = [ pkgs.nodejs ]; } ''
            node --import ${self}/extensions/sandbox/test-setup.ts --test \
              ${self}/extensions/sandbox/background-jobs.test.ts \
              ${self}/extensions/sandbox/broker-client.test.ts \
              ${self}/extensions/sandbox/broker-policy.test.ts \
              ${self}/extensions/sandbox/codex-command.test.ts \
              ${self}/extensions/sandbox/development-caches.test.ts \
              ${self}/extensions/sandbox/io-permissions.test.ts \
              ${self}/extensions/sandbox/io-policy.test.ts \
              ${self}/extensions/sandbox/native-denials.test.ts \
              ${self}/extensions/sandbox/native-sandbox-ops.test.ts \
              ${self}/extensions/sandbox/permission-system-approval.test.ts \
              ${self}/extensions/sandbox/sandbox-failures.test.ts
            touch "$out"
          '';

          sandbox-broker = pkgs.callPackage ./nix/pi-sandbox-broker.nix { };
          mcp-cli = pkgs.runCommand "pi-mcp-cli-test" { nativeBuildInputs = [ mcpCli ]; } ''
            test "$(mcp-cli --version)" = "mcp-cli v0.3.0"
            touch "$out"
          '';
          dense-tools = pkgs.runCommand "pi-dense-tools-test" { nativeBuildInputs = [ denseTools ]; } ''
            mkdir before after empty-before empty-after
            printf '%s\n' '{ lib, ... }:' 'let' '  oldValue = "before";' 'in' '{ inherit oldValue; }' > before/sample.nix
            printf '%s\n' '{ lib, ... }:' 'let' '  newValue = "after";' 'in' '{ inherit newValue; }' > after/sample.nix

            pi-diff --width=140 before after > split
            grep -F 'sample.nix' split
            grep -F '│' split
            grep -F $'\033[38;2;251;73;52' split

            pi-diff --width=80 before after > unified
            grep -F 'sample.nix' unified
            if grep -F '│' unified; then
              echo "narrow diffs must use the unified layout" >&2
              exit 1
            fi

            pi-diff empty-before empty-after > empty
            test ! -s empty
            touch "$out"
          '';
          permission-system = pkgs.callPackage ./nix/pi-permission-system.nix { };
          web-access = webAccess;

          governance = pkgs.runCommand "pi-governance-tests" { nativeBuildInputs = [ pkgs.nodejs ]; } ''
            node --test \
              ${self}/tests/governance.test.ts \
              ${self}/tests/theme-and-rendering.test.ts \
              ${self}/tests/terminal-text.test.ts
            touch "$out"
          '';
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt-tree);
    };
}
