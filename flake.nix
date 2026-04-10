{
  description = "Nix flake for the smartedit Rust CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    let
      cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      workspacePackage = cargoToml.workspace.package;
      pname = "smartedit";
      version = workspacePackage.version;
    in
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;

        smartedit = pkgs.rustPlatform.buildRustPackage {
          inherit pname version;

          src = lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;

          cargoBuildFlags = [ "--package" "smartedit-cli" ];
          cargoTestFlags = [ "--package" "smartedit-cli" ];

          meta = with lib; {
            description = workspacePackage.description;
            homepage = workspacePackage.repository;
            license = licenses.mit;
            mainProgram = "smartedit";
            platforms = platforms.all;
          };
        };

        smarteditApp = flake-utils.lib.mkApp {
          drv = smartedit;
          exePath = "/bin/smartedit";
        };
      in
      {
        packages = {
          inherit smartedit;
          default = smartedit;
        };

        apps = {
          smartedit = smarteditApp;
          default = smarteditApp;
        };

        checks = {
          inherit smartedit;
          default = smartedit;
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            clippy
            rust-analyzer
            rustc
            rustfmt
          ];
        };

        formatter = pkgs.nixpkgs-fmt;
      })
    // {
      overlays.default = final: prev: {
        smartedit = self.packages.${final.system}.default;
      };
    };
}
