{
  description = "flake for rust projects";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" ];
    in
    {
      devShell = builtins.listToAttrs (map
        (system: {
          name = system;
          value =
            let
              pkgs = nixpkgs.legacyPackages.${system};
            in
            pkgs.mkShell {
              buildInputs = with pkgs; [
                rustc
                cargo
                clippy
                rust-analyzer
                rustfmt
                bacon
              ];


              shellHook = ''
                export LD_LIBRARY_PATH="${pkgs.stdenv.cc.cc.lib.outPath}/lib:$LD_LIBRARY_PATH"
              '';
            };
        })
        systems);
    };
}

