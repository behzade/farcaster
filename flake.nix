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
        "aarch64-linux"
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
          sandbox = pkgs.callPackage ./nix/pi-sandbox-extension.nix { };
          subagents = pkgs.callPackage ./nix/pi-subagents.nix { };
          openaiServerCompaction = pkgs.callPackage ./nix/pi-openai-server-compaction.nix { };
        in
        {
          inherit sandbox subagents;
          openai-server-compaction = openaiServerCompaction;
          default = sandbox;
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          sandbox-tests = pkgs.runCommand "pi-sandbox-tests" { nativeBuildInputs = [ pkgs.nodejs ]; } ''
            node --test \
              ${self}/extensions/sandbox/codex-command.test.ts \
              ${self}/extensions/sandbox/io-permissions.test.ts \
              ${self}/extensions/sandbox/io-policy.test.ts
            touch "$out"
          '';

          governance = pkgs.runCommand "pi-governance-tests" { nativeBuildInputs = [ pkgs.nodejs ]; } ''
            node --test ${self}/tests/governance.test.ts
            touch "$out"
          '';
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt-tree);
    };
}
