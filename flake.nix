{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    devshell = {
      url = "github:numtide/devshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
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
            libX11
            libXcursor
            libXi
            libXrandr
            wayland
            libxkbcommon
            fontconfig
            freetype
            expat
          ];

          audioLibs = with pkgs; [
            alsa-lib
            libopus
          ];

          pgStart = ''
            if ! pg_ctl status -D "$PGDATA" >/dev/null 2>&1; then
              (
                for fd in /proc/$$/fd/*; do
                  n=''${fd##*/}
                  [ "$n" -gt 2 ] 2>/dev/null && eval "exec $n>&-"
                done
                pg_ctl start -D "$PGDATA" -l "$PGDATA/postgres.log" -o "-k $PGHOST"
              )
            fi

            until pg_isready -h "$PGHOST" -q; do sleep 0.1; done

            ensure() { 
                psql -h "$PGHOST" -d postgres -tAc "$1" | grep -q 1 || psql -h "$PGHOST" -d postgres -c "$2"; 
            }
            ensure "SELECT 1 FROM pg_roles WHERE rolname='postgres'" "CREATE ROLE postgres WITH SUPERUSER LOGIN"
            ensure "SELECT 1 FROM pg_database WHERE datname='local'" "CREATE DATABASE local OWNER postgres"
          '';
        in
        {
          devshells.default = {
            packages = [
              # rust toolchain
              (inputs.rust-overlay.lib.mkRustBin { } pkgs).stable.latest.default
              pkgs.rust-analyzer

              # c toolchain + linker
              pkgs.stdenv.cc
              pkgs.mold

              # native build deps
              pkgs.pkg-config
              pkgs.openssl

              # database
              pkgs.postgresql

              # formatters
              pkgs.taplo
              pkgs.yamlfmt

              pkgs.opentofu
            ]
            ++ guiLibs;

            devshell.startup.postgres.text = ''
              mkdir -p "$PGHOST"
              if [ ! -d "$PGDATA" ]; then
                initdb --auth=trust --no-locale --encoding=UTF8
              fi
              ${pgStart}
            '';

            env = [
              {
                name = "PGDATA";
                eval = "$PWD/.pgdata";
              }
              {
                name = "PGHOST";
                eval = "$PWD/.pgsocket";
              }
              {
                name = "RUSTFLAGS";
                value = "-C link-arg=-fuse-ld=mold";
              }
              {
                name = "LD_LIBRARY_PATH";
                value = pkgs.lib.makeLibraryPath (guiLibs ++ audioLibs);
              }
              {
                name = "PKG_CONFIG_PATH";
                value = pkgs.lib.makeSearchPathOutput "dev" "lib/pkgconfig" (
                  [ pkgs.openssl ] ++ guiLibs ++ audioLibs
                );
              }
              {
                name = "RUST_BACKTRACE";
                value = "0";
              }
              {
                name = "RUST_LOG";
                value = "debug";
              }
              {
                name = "JWT_KEY";
                value = "c2ce2a9e1b3f3c0c02dc11c49c868e154efbede9e46faa47c0bbef01af5a5e00";
              }
            ];

            commands = [
              { package = pkgs.tokei; }
              { package = pkgs.lazygit; }
              { package = pkgs.postgresql; }
              { package = pkgs.secretspec; }
              {
                name = "pg-start";
                command = pgStart;
              }
              {
                name = "pg-stop";
                command = ''pg_ctl stop -D "$PGDATA"'';
              }
              {
                name = "pg-status";
                command = ''pg_ctl status -D "$PGDATA"'';
              }
            ];
          };
        };

      packages.client = pkgs.rustPlatform.buildRustPackage {
        pname = "chat-rs-client";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        buildAndTestSubdir = "crates/client";   # or cargoBuildFlags = [ "-p" "chat-client" ]

        nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper ];
        buildInputs = [ pkgs.alsa-lib pkgs.libopus ];

        postFixup = ''
          wrapProgram $out/bin/client \
            --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath guiLibs} \
            --set ALSA_CONFIG_PATH "${pkgs.alsa-lib}/share/alsa/alsa.conf:${alsaConf}"
        '';
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
