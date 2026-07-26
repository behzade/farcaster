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
          guardian = pkgs.callPackage ./nix/pi-guardian.nix { };
          sandbox = pkgs.callPackage ./nix/pi-sandbox-extension.nix { };
          subagents = pkgs.callPackage ./nix/pi-subagents.nix { };
          openaiServerCompaction = pkgs.callPackage ./nix/pi-openai-server-compaction.nix { };
        in
        {
          inherit guardian sandbox subagents;
          openai-server-compaction = openaiServerCompaction;
          default = guardian;
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          guardian-tests = pkgs.buildNpmPackage {
            pname = "pi-guardian-tests";
            version = "1.0.0";
            src = ./extensions/guardian;
            npmDepsHash = "sha256-6XZSjIW0APmtD0jcIJYaQCJx4EjYPELU4vTJ5gD3FTE=";
            npmDepsFetcherVersion = 2;
            dontNpmBuild = true;

            buildPhase = ''
              runHook preBuild
              npm test
              runHook postBuild
            '';

            installPhase = ''
              runHook preInstall
              touch "$out"
              runHook postInstall
            '';
          };

          governance = pkgs.runCommand "pi-governance-tests" { nativeBuildInputs = [ pkgs.nodejs ]; } ''
            node --test ${self}/tests/governance.test.ts
            touch "$out"
          '';
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt-tree);
    };
}
