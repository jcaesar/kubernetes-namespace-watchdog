{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
	crane = {
      url = "github:ipetkov/crane";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        #rust-overlay.follows = "rust-overlay";
        #flake-utils.follows = "flake-utils";
      };
    };
  };
  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane }:
    flake-utils.lib.eachDefaultSystem
      (system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
          };
          craneLib = crane.lib.${system};
          src = craneLib.cleanCargoSource ./.;
          mkBins = commonArgsOrig:
            let
              commonArgs = commonArgsOrig // { inherit src; };
              bin = craneLib.buildPackage (commonArgs // { 
                #cargoArtifacts = craneLib.buildDepsOnly commonArgs;
                cargoBuildCommand = "cargo build --profile maxopt";
              });
            in bin;
          allBins = mkBins {
            pname = "workspace";
          };
          watcherBin = mkBins {
            pname = "kubernetes-namespace-watchdog-watcher";
            cargoExtraArgs = "-p kubernetes-namespace-watchdog-watcher";
          };
          dockerImage = pkgs.dockerTools.streamLayeredImage {
            name = "liftm/kubernetes-namespace-watchdog";
            tag = "latest";
            contents = [ watcherBin pkgs.busybox ];
            config = {
              Cmd = [ "${watcherBin}/bin/kubernetes-namespace-watchdog-watcher" ];
            };
          };
        in
        with pkgs;
        {
          packages =
            {
              inherit dockerImage;
              default = allBins;
            };
          devShells.default = mkShell {
            inputsFrom = [ allBins ];
          };
        }
      );
}
