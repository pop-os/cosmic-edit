{
  description = "Text Editor application for the COSMIC desktop environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    nix-filter.url = "github:numtide/nix-filter";
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, nix-filter, crane, fenix }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        craneLib = crane.lib.${system}.overrideToolchain fenix.packages.${system}.stable.toolchain;
        crateNameFromCargoToml = craneLib.crateNameFromCargoToml {cargoToml = ./Cargo.toml;};
        pkgDef = {
          inherit (crateNameFromCargoToml) pname version;
          src = nix-filter.lib.filter {
            root = ./.;
            include = [
              ./src
              ./Cargo.toml
              ./Cargo.lock
              ./i18n
              ./i18n.toml
              ./justfile
            ];
          };
          nativeBuildInputs = with pkgs; [
            just
            pkg-config
            autoPatchelfHook
          ];
          buildInputs = with pkgs; [
            systemdMinimal
            bashInteractive
            libxkbcommon
            freetype
            fontconfig
            expat
            lld
            desktop-file-utils
            stdenv.cc.cc.lib
            libinput
           ];
        };

        cargoArtifacts = craneLib.buildDepsOnly pkgDef;
        cosmic-edit= craneLib.buildPackage (pkgDef // {
          inherit cargoArtifacts;
        });
      in {
        checks = {
          inherit cosmic-edit;
        };

        apps.default = flake-utils.lib.mkApp {
          drv = cosmic-edit;
        };
        packages.default = cosmic-edit.overrideAttrs (oldAttrs: rec {
          installPhase = ''
            just prefix=$out install
          '';
        });

        devShells.default = pkgs.mkShell rec {
          inputsFrom = builtins.attrValues self.checks.${system};
          LD_LIBRARY_PATH = pkgs.lib.strings.makeLibraryPath (builtins.concatMap (d: d.runtimeDependencies) inputsFrom);
        };
      });

  nixConfig = {
    # Cache for the Rust toolchain in fenix
    extra-substituters = [ "https://nix-community.cachix.org" ];
    extra-trusted-public-keys = [ "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs=" ];
  };
}
