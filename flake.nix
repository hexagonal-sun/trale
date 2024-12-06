{
  inputs = {
    fenix.url = "github:nix-community/fenix";
    utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, utils, fenix }:
    utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        toolchain = fenix.packages.${system}.latest.toolchain;
      in {
        devShell = pkgs.mkShell {
            buildInputs = [ toolchain ];
          };
      });
}
