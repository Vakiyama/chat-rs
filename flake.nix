{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    devshell.url = "github:numtide/devshell";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    bacon-ls = {
      url = "github:crisidev/bacon-ls";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    inputs@{ self, ... }:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [ inputs.devshell.flakeModule ];

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];

      perSystem =
        { pkgs, system, ... }:
        let
          guiLibs = with pkgs; [
            libGL
            vulkan-loader
            xorg.libX11
            xorg.libXcursor
            xorg.libXi
            xorg.libXrandr
            wayland
            libxkbcommon
            fontconfig
            freetype
            expat
          ];
        in
        {
          devshells.default = {
            packages = [
              (inputs.rust-overlay.lib.mkRustBin { } pkgs).stable.latest.default
              pkgs.rust-analyzer
              pkgs.bacon
              pkgs.taplo
              inputs.bacon-ls.defaultPackage.${system}
              pkgs.mold
              pkgs.pkg-config
              pkgs.lazygit
              pkgs.websocat
              pkgs.diesel-cli
              pkgs.openssl
              pkgs.watchexec
              pkgs.openssl
            ]
            ++ guiLibs;

            env = [
              {
                name = "RUSTFLAGS";
                value = "-C link-arg=-fuse-ld=mold";
              }
              {
                name = "LD_LIBRARY_PATH";
                value = pkgs.lib.makeLibraryPath guiLibs;
              }
              {
                name = "PKG_CONFIG_PATH";
                value = "${pkgs.openssl.dev}/lib/pkgconfig";
              }
              {
                name = "RUST_BACKTRACE";
                value = "0";
              }
            ];
          };
        };
    };

  nixConfig = {
    extra-substituters = [
      "https://nix-community.cachix.org?priority=1"
      "https://numtide.cachix.org?priority=2"
    ];
    extra-trusted-public-keys = [
      "nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="
      "numtide.cachix.org-1:2ps1kLBUWjxIneOy1Ik6cQjb41X0iXVXeHigGmycPPE="
    ];
  };
}
