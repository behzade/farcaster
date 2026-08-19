{
  description = "Behzad's reviewed Pi coding-agent extensions";

  inputs.crane.url = "github:ipetkov/crane";
  inputs.guardian = {
    url = "github:behzade/pi-guardian";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs =
    {
      crane,
      guardian,
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
          coreExtensions = pkgs.callPackage ./nix/pi-core-extensions.nix { };
          mcpCli = pkgs.callPackage ./nix/pi-mcp-cli.nix { };
          sandboxBroker = guardian.packages.${system}.sandbox-broker;
          sandbox = guardian.packages.${system}.guardian;
          denseTools = pkgs.callPackage ./nix/pi-dense-tools.nix { };
          sessionAgents = pkgs.callPackage ./nix/pi-session-agents.nix { };
          webAccess = pkgs.callPackage ./nix/pi-web-access.nix { };
          openaiServerCompaction = pkgs.callPackage ./nix/pi-openai-server-compaction.nix { };
          projectTools = pkgs.callPackage ./nix/pi-project-tools.nix { };
          piTerminal = pkgs.callPackage ./nix/pi-terminal.nix { };
          piGpui = pkgs.callPackage ./nix/pi-gpui.nix {
            craneLib = crane.mkLib pkgs;
            inherit piTerminal;
          };
          pi = pkgs.symlinkJoin {
            name = "pi";
            paths = [
              piTerminal
              piGpui
            ];
            meta = {
              description = "Pi terminal and native GUI clients";
              mainProgram = "pi";
              platforms = pkgs.lib.platforms.darwin ++ pkgs.lib.platforms.linux;
            };
          };
          agent = pkgs.callPackage ./nix/pi-agent.nix {
            inherit
              coreExtensions
              denseTools
              openaiServerCompaction
              piTerminal
              projectTools
              sandbox
              sessionAgents
              webAccess
              ;
          };
        in
        {
          inherit agent sandbox;
          session-agents = sessionAgents;
          core-extensions = coreExtensions;
          inherit pi;
          pi-terminal = piTerminal;
          mcp-cli = mcpCli;
          sandbox-broker = sandboxBroker;
          dense-tools = denseTools;
          openai-server-compaction = openaiServerCompaction;
          project-tools = projectTools;
          web-access = webAccess;
          default = agent;
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          nativeRuntimeLibraries =
            with pkgs;
            lib.optionals stdenv.isLinux [
              alsa-lib
              at-spi2-atk
              cairo
              cups
              dbus
              expat
              fontconfig
              freetype
              glib
              gtk3
              libdrm
              libgbm
              libGL
              libx11
              libxcb
              libxcomposite
              libxdamage
              libxext
              libxfixes
              libxkbcommon
              libxkbfile
              libxrandr
              libxshmfence
              nspr
              nss
              pango
              pciutils
              stdenv.cc.cc
              systemd
              vulkan-loader
              wayland
            ];
        in
        rec {
          pi-gpui = pkgs.mkShell {
            buildInputs = nativeRuntimeLibraries;
            packages = with pkgs; [
              cargo
              clippy
              git
              nix
              nixfmt-tree
              nodejs
              pkg-config
              rust-analyzer
              rustc
              rustfmt
              rustPlatform.bindgenHook
            ];
            shellHook = ''
              export CARGO_TARGET_DIR="$PWD/target"
              ${pkgs.lib.optionalString pkgs.stdenv.isLinux ''
                export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath nativeRuntimeLibraries}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
                export NIX_LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath nativeRuntimeLibraries}''${NIX_LD_LIBRARY_PATH:+:$NIX_LD_LIBRARY_PATH}"
              ''}
            '';
          };
          default = pi-gpui;
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          coreExtensions = pkgs.callPackage ./nix/pi-core-extensions.nix { };
          denseTools = pkgs.callPackage ./nix/pi-dense-tools.nix { };
          mcpCli = pkgs.callPackage ./nix/pi-mcp-cli.nix { };
          sandboxBroker = guardian.packages.${system}.sandbox-broker;
          sandbox = guardian.packages.${system}.guardian;
          openaiServerCompaction = pkgs.callPackage ./nix/pi-openai-server-compaction.nix { };
          projectTools = pkgs.callPackage ./nix/pi-project-tools.nix { };
          piTerminal = pkgs.callPackage ./nix/pi-terminal.nix { };
          piGpui = pkgs.callPackage ./nix/pi-gpui.nix {
            craneLib = crane.mkLib pkgs;
            inherit piTerminal;
          };
          sessionAgents = pkgs.callPackage ./nix/pi-session-agents.nix { };
          webAccess = pkgs.callPackage ./nix/pi-web-access.nix { };
          agent = pkgs.callPackage ./nix/pi-agent.nix {
            inherit
              coreExtensions
              denseTools
              openaiServerCompaction
              piTerminal
              projectTools
              sandbox
              sessionAgents
              webAccess
              ;
          };
        in
        {
          agent-extension-layout = pkgs.runCommand "pi-agent-extension-layout-test" { } ''
            test -f ${agent}/extensions/node_modules/effect/package.json
            test "$(readlink ${agent}/extensions/node_modules)" = ${coreExtensions}/node_modules
            test -L ${agent}/prompts
            test -f ${agent}/prompts/commit.md
            touch "$out"
          '';
          core-extensions = pkgs.runCommand "pi-core-extensions-test" { nativeBuildInputs = [ pkgs.nodejs ]; } ''
            cp -R ${coreExtensions}/* .
            chmod -R u+w node_modules
            mkdir -p node_modules/@earendil-works
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/@earendil-works/pi-ai node_modules/@earendil-works/pi-ai
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/@earendil-works/pi-coding-agent node_modules/@earendil-works/pi-coding-agent
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/typebox node_modules/typebox
            timeout 60 node --experimental-strip-types -e '
              await Promise.all([
                import("./agent-feedback.ts"),
                import("./notifications.ts"),
                import("./prompt-inspector.ts"),
                import("./session-hooks.ts"),
                import("./title-state.ts"),
                import("./user-input.ts"),
              ])
            '
            touch "$out"
          '';

          sandbox-tests = pkgs.runCommand "pi-sandbox-tests" { nativeBuildInputs = [ pkgs.nodejs ]; } ''
            cp ${guardian}/extensions/sandbox/*.ts .
            cp -R ${sandbox}/node_modules .
            chmod -R u+w node_modules
            mkdir -p node_modules/@earendil-works
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/@earendil-works/pi-ai node_modules/@earendil-works/pi-ai
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/@earendil-works/pi-coding-agent node_modules/@earendil-works/pi-coding-agent
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/typebox node_modules/typebox
            node --import ./test-setup.ts --test \
              background-jobs.test.ts \
              broker-client.test.ts \
              broker-policy.test.ts \
              native-background-jobs.test.ts \
              native-network-proxy.test.ts \
              network-policy.test.ts \
              sandbox-config.test.ts \
              development-caches.test.ts \
              extension-schema.test.ts \
              io-permissions.test.ts \
              io-policy.test.ts \
              native-sandbox-ops.test.ts \
              project-policy.test.ts \
              approval-transport.test.ts
            touch "$out"
          '';

          sandbox-broker = sandboxBroker;
          mcp-cli = pkgs.runCommand "pi-mcp-cli-test" { nativeBuildInputs = [ mcpCli ]; } ''
            test "$(mcp-cli --version)" = "mcp-cli v0.3.0"
            touch "$out"
          '';
          dense-tools = pkgs.runCommand "pi-dense-tools-test" { nativeBuildInputs = [ denseTools ]; } ''
            mkdir before after empty-before empty-after
            printf '%s\n' '{ lib, ... }:' 'let' '  oldValue = "before";' 'in' '{ inherit oldValue; }' > before/sample.nix
            printf '%s\n' '{ lib, ... }:' 'let' '  newValue = "after";' 'in' '{ inherit newValue; }' > after/sample.nix

            PI_DIFF_CACHE_DIR="$PWD/cache" PI_DIFF_CACHE_TRACE=1 pi-diff --width=140 before after > split 2> first-cache
            grep -F 'pi-diff cache miss' first-cache
            grep -F 'sample.nix' split
            grep -F '│' split
            grep -F $'\033[38;2;251;73;52' split

            PI_DIFF_CACHE_DIR="$PWD/cache" PI_DIFF_CACHE_TRACE=1 pi-diff --width=140 before after > split-cached 2> second-cache
            grep -F 'pi-diff cache hit' second-cache
            cmp split split-cached

            PI_DIFF_CACHE_DIR="$PWD/cache" pi-diff --width=80 before after > unified
            grep -F 'sample.nix' unified
            if grep -F '│' unified; then
              echo "narrow diffs must use the unified layout" >&2
              exit 1
            fi

            pi-diff empty-before empty-after > empty
            test ! -s empty
            touch "$out"
          '';
          pi-terminal = pkgs.runCommand "pi-terminal-test" { nativeBuildInputs = [ piTerminal ]; } ''
            test "$(pi --version)" = "0.84.2"
            touch "$out"
          '';
          pi-gpui = piGpui;
          openai-server-compaction-tests = pkgs.runCommand "pi-openai-server-compaction-tests" {
            nativeBuildInputs = [ pkgs.nodejs ];
          } ''
            cp -R ${openaiServerCompaction}/src ${openaiServerCompaction}/node_modules .
            chmod -R u+w node_modules
            mkdir -p node_modules/@earendil-works test
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/@earendil-works/pi-agent-core node_modules/@earendil-works/pi-agent-core
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/@earendil-works/pi-ai node_modules/@earendil-works/pi-ai
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/@earendil-works/pi-coding-agent node_modules/@earendil-works/pi-coding-agent
            cp ${self}/extensions/openai-server-compaction/test/*.test.ts test/
            timeout 60 node --experimental-strip-types -e 'import("./src/index.ts")'
            timeout 60 node --experimental-strip-types --test test/openai-ws-connection.test.ts
            timeout 60 node --experimental-strip-types --test test/openai-ws-stream.test.ts
            timeout 60 node --experimental-strip-types --test test/continuation-compaction.test.ts
            touch "$out"
          '';
          project-tools-tests = pkgs.runCommand "pi-project-tools-tests" {
            nativeBuildInputs = [ pkgs.nodejs ];
          } ''
            cp -R ${projectTools}/src ${projectTools}/node_modules .
            mkdir test
            cp ${self}/extensions/project-tools/test/project-tools.test.ts test/
            node --experimental-strip-types --test test/project-tools.test.ts
            touch "$out"
          '';
          session-agents-tests = pkgs.runCommand "pi-session-agents-tests" {
            nativeBuildInputs = [ pkgs.nodejs ];
          } ''
            cp -R ${sessionAgents}/* .
            chmod -R u+w .
            cp ${self}/extensions/subagents/adapter.test.ts ${self}/extensions/subagents/core.test.ts .
            mkdir -p node_modules/@earendil-works
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/@earendil-works/pi-agent-core node_modules/@earendil-works/pi-agent-core
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/@earendil-works/pi-ai node_modules/@earendil-works/pi-ai
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/@earendil-works/pi-coding-agent node_modules/@earendil-works/pi-coding-agent
            ln -s ${piTerminal}/lib/pi-terminal/node_modules/typebox node_modules/typebox
            node --test adapter.test.ts core.test.ts
            node -e 'import("./index.ts")'
            node --test ${self}/tests/session-agents-package.test.ts
            touch "$out"
          '';
          web-access = webAccess;

          governance = pkgs.runCommand "pi-governance-tests" { nativeBuildInputs = [ pkgs.nodejs ]; } ''
            node --test \
              ${self}/tests/agent-feedback.test.ts \
              ${self}/tests/governance.test.ts \
              ${self}/tests/session-agents-package.test.ts \
              ${self}/tests/prompt-contract.test.ts \
              ${self}/tests/prompt-inspector.test.ts \
              ${self}/tests/theme-and-rendering.test.ts \
              ${self}/tests/theme-selection.test.ts \
              ${self}/tests/tui-only.test.ts \
              ${self}/tests/terminal-text.test.ts
            touch "$out"
          '';
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt-tree);
    };
}
