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
    crane.url = "github:ipetkov/crane";
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
          rustToolchain = (inputs.rust-overlay.lib.mkRustBin { } pkgs).stable.latest.default;
          craneLib = (inputs.crane.mkLib pkgs).overrideToolchain rustToolchain;

          # crane's cargo filter strips the .proto files build.rs compiles
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            name = "chat-rs-source";
            filter = path: type: (pkgs.lib.hasSuffix ".proto" path) || (craneLib.filterCargoSources path type);
          };

          commonArgs = {
            inherit src;
            version = "0.1.0";
            strictDeps = true;
            PROTOC = "${pkgs.protobuf}/bin/protoc";
            nativeBuildInputs = [
              pkgs.pkg-config
              pkgs.protobuf
              # cmake is a build-time tool (some -sys build scripts invoke it), so it
              # must be on PATH. With strictDeps only nativeBuildInputs land there —
              # in buildInputs it's treated as a target lib and never found.
              pkgs.cmake
            ];
            buildInputs = [
              pkgs.openssl
              pkgs.alsa-lib
              pkgs.libopus
            ];
          };

          cargoArtifacts = craneLib.buildDepsOnly (
            commonArgs
            // {
              pname = "chat-rs-deps";
              # buildDepsOnly's dummy src omits the .proto files build.rs needs
              postPatch = ''
                cp -r ${src}/crates/shared/src/proto crates/shared/src/proto
              '';
            }
          );

          serverUrl = "http://5.78.193.193:3000";

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

          alsaConf = pkgs.writeText "asound.conf" ''
            pcm_type.pipewire {
              lib "${pkgs.pipewire}/lib/alsa-lib/libasound_module_pcm_pipewire.so"
            }
            ctl_type.pipewire {
              lib "${pkgs.pipewire}/lib/alsa-lib/libasound_module_ctl_pipewire.so"
            }
            pcm.!default { type pipewire }
            ctl.!default { type pipewire }
          '';

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
              pkgs.protobuf
              pkgs.cmake

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
                value = "0000000000000000000000000000000000000000000000000000000000000000";
              }
              {
                name = "PROTOC";
                eval = "${pkgs.protobuf}/bin/protoc";
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

          packages = {
            server = craneLib.buildPackage (
              commonArgs
              // {
                inherit cargoArtifacts;
                pname = "chat-rs-server";
                cargoExtraArgs = "--locked --package chat-server";
                doCheck = false;
                meta.mainProgram = "server";
              }
            );

            client-linux = craneLib.buildPackage (
              commonArgs
              // {
                inherit cargoArtifacts;
                pname = "chat-rs-client";
                cargoExtraArgs = "--locked --package chat-client";
                doCheck = false;
                meta.mainProgram = "client";
                DEFAULT_SERVER_URL = serverUrl;
                nativeBuildInputs = commonArgs.nativeBuildInputs ++ [ pkgs.makeWrapper ];
                postInstall = ''
                  wrapProgram $out/bin/client \
                    --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath guiLibs} \
                    --set ALSA_CONFIG_PATH "${pkgs.alsa-lib}/share/alsa/alsa.conf:${alsaConf}"
                '';
              }
            );
          }
          // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            client-windows =
              let
                crossPkgs = pkgs.pkgsCross.mingwW64;
                rustToolchainWin = (inputs.rust-overlay.lib.mkRustBin { } pkgs).stable.latest.default.override {
                  targets = [ "x86_64-pc-windows-gnu" ];
                };
                craneLibWin = (inputs.crane.mkLib pkgs).overrideToolchain rustToolchainWin;
                opusStatic = crossPkgs.libopus.overrideAttrs (old: {
                  mesonFlags = (old.mesonFlags or [ ]) ++ [ "-Ddefault_library=static" ];
                });
              in
              craneLibWin.buildPackage {
                inherit src;
                DEFAULT_SERVER_URL = serverUrl;
                version = "0.1.0";
                strictDeps = true;
                pname = "chat-rs-client-windows";
                cargoExtraArgs = "--locked --package chat-client";
                doCheck = false;

                CARGO_BUILD_TARGET = "x86_64-pc-windows-gnu";

                nativeBuildInputs = [
                  pkgs.pkg-config
                  pkgs.protobuf
                  pkgs.cmake
                ];
                depsBuildBuild = [ crossPkgs.stdenv.cc ];
                buildInputs = [
                  opusStatic
                  crossPkgs.windows.pthreads
                ];

                PROTOC = "${pkgs.protobuf}/bin/protoc";
                # audiopus_sys would build opus from source (wrong arch) unless pointed at a prebuilt static lib
                OPUS_STATIC = "1";
                OPUS_NO_PKG = "1";
                OPUS_LIB_DIR = "${opusStatic}/lib";

                # rust's windows-gnu target links -l:libpthread.a from winpthreads
                CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-L native=${crossPkgs.windows.pthreads}/lib";

                # ring's cc-rs build looks up the compiler by rust triple, which nix doesn't export
                "CC_x86_64-pc-windows-gnu" = "${crossPkgs.stdenv.cc}/bin/${crossPkgs.stdenv.cc.targetPrefix}cc";
                "AR_x86_64-pc-windows-gnu" =
                  "${crossPkgs.stdenv.cc.bintools.bintools}/bin/${crossPkgs.stdenv.cc.targetPrefix}ar";
              };
          };

          # webrtc tests need routable interfaces for ICE but glorious nix sandbox has only
          # loopback, so skip them here
          checks.tests = craneLib.cargoTest (
            commonArgs
            // {
              inherit cargoArtifacts;
              pname = "chat-rs-tests";
              cargoExtraArgs = "--locked --package chat-server";
              cargoTestExtraArgs = "-- --skip library::webrtc";
              nativeCheckInputs = [ pkgs.postgresql ];
              ENV = "DEV";
              JWT_KEY = "0000000000000000000000000000000000000000000000000000000000000000";
              RESEND_API_KEY = "test";
              DB_CONNECTION = "postgres://postgres@localhost:5432/local";
              preCheck = ''
                export PGDATA=$(mktemp -d) PGHOST=$(mktemp -d)
                initdb --auth=trust --no-locale --encoding=UTF8 -U postgres >/dev/null
                pg_ctl start -D "$PGDATA" -w -o "-k $PGHOST -h localhost -p 5432" >/dev/null
                createdb -h localhost -p 5432 -U postgres local
              '';
              postCheck = ''
                pg_ctl stop -D "$PGDATA" -m immediate || true
              '';
            }
          );
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
