# ============================================================================
# Strobe on Nix: a dev shell, and the `strobe` binary as a package.
#
#   nix develop            build and test with the repo's own cargo
#   nix develop .#software same, plus Mesa's software GL (no GPU, CI, a VM)
#   nix build              ./result/bin/strobe, wrapped so it finds its libs
#   nix run                build it and start it
#
# Why each dependency is here is in docs/DECISIONS.md (D295) and README.md's
# Linux section. The short version: ALSA is the only library the linker sees
# (cpal and midir). Everything that draws, windows, or reads the keyboard is
# opened with dlopen at run time (winit, glutin, xkbcommon-dl, x11-dl), so on
# Nix it must be on LD_LIBRARY_PATH, or the window never opens.
#
# The Rust toolchain is nixpkgs' own cargo and rustc, not rust-toolchain.toml:
# the pin there is 1.98.0, nixpkgs-unstable carries 1.98.x, and the workspace's
# `rust-version` is 1.98. If nixpkgs ever falls behind the pin, swap in
# rust-overlay's `rust-bin.fromRustupToolchainFile ./rust-toolchain.toml`.
# ============================================================================
{
  description = "Strobe: an audio-reactive VJ engine";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      # Strobe is MIT with the Commons Clause (D486), which nixpkgs counts as
      # unfree. This flake's own package set may build that one package, so
      # `nix build` works without NIXPKGS_ALLOW_UNFREE; nothing else is let in.
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              config.allowUnfreePredicate = pkg: (pkg.pname or "") == "strobe";
            }
          )
        );

      # Opened with dlopen, so no build step sees them and nothing records
      # them in the binary. winit and glutin name each library by its
      # soname at run time; LD_LIBRARY_PATH is how Nix lets them be found.
      runtimeLibs =
        pkgs: with pkgs; [
          libxkbcommon # xkbcommon-dl: libxkbcommon.so.0 and libxkbcommon-x11.so.0
          wayland # winit and glutin: libwayland-client, -cursor, -egl
          libGL # glvnd's libGL.so.1, libEGL.so.1, libGLX.so.0; the driver comes from the system
          libx11 # x11-dl: libX11.so.6
          libxcursor # winit: libXcursor.so.1
          libxi # winit: libXi.so.6, XInput2 for the pointer and tablets
          libxrandr # winit: libXrandr.so.2, monitor modes and refresh rate
          libxcb # x11-dl and x11rb: libxcb.so.1
        ];
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          # `stdenv`'s C compiler comes with mkShell: QuickJS (rquickjs-sys)
          # and ring are C.
          nativeBuildInputs = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            pkg-config
            git # build.rs reads the commit; `cargo xtask changelog` walks history
          ];
          # Linked, so it needs headers and a .pc file: cpal and midir use ALSA.
          buildInputs = [ pkgs.alsa-lib ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibs pkgs ++ [ pkgs.alsa-lib ]);
        };

        # Mesa's software rasteriser and drivers for a machine with no GL of
        # its own: a CI container, a VM, a quick `strobe-bench` run. It
        # replaces the system driver, so on a real GPU use `default`.
        software = pkgs.mkShell {
          inputsFrom = [ self.devShells.${pkgs.stdenv.hostPlatform.system}.default ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (
            runtimeLibs pkgs
            ++ [
              pkgs.alsa-lib
              pkgs.mesa
            ]
          );
          __EGL_VENDOR_LIBRARY_DIRS = "${pkgs.mesa}/share/glvnd/egl_vendor.d";
          LIBGL_DRIVERS_PATH = "${pkgs.mesa}/lib/dri";
        };
      });

      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "strobe";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

          src = pkgs.lib.cleanSource ./.;
          # Every dependency is a crates.io one, so the lock file is enough.
          # A git dependency would need `outputHashes` here.
          cargoLock.lockFile = ./Cargo.lock;

          # The app and the benchmark, built together so they share one
          # feature resolution and one compile of the dependency tree.
          cargoBuildFlags = [
            "--package"
            "strobe-app"
            "--package"
            "strobe-bench"
          ];
          # The test suite needs a GL context and walks the source tree; the
          # CI Linux job runs it. This package is for running Strobe.
          doCheck = false;

          nativeBuildInputs = with pkgs; [
            pkg-config
            makeWrapper
          ];
          buildInputs = [ pkgs.alsa-lib ];

          # The build script asks `git` for the commit, and a Nix build has
          # no repository, so say it here: `strobe --diagnose` then names it.
          env.STROBE_GIT_HASH = self.shortRev or self.dirtyShortRev or "unknown";

          postInstall = ''
            # The scene library, found "beside the program" (library::resolve):
            # `strobe` run from any folder lists it.
            mkdir -p $out/share/strobe
            cp -r scenes $out/share/strobe/scenes
            ln -s ../share/strobe/scenes $out/bin/scenes

            for bin in strobe strobe-bench strobe-corpus; do
              wrapProgram $out/bin/$bin \
                --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (runtimeLibs pkgs)}
            done
          '';

          meta = {
            description = "Audio-reactive VJ engine for GLSL scenes";
            homepage = "https://github.com/lnorton89/strobe";
            license = {
              fullName = "MIT with the Commons Clause";
              url = "https://commonsclause.com/";
              free = false;
              redistributable = true;
            };
            mainProgram = "strobe";
            platforms = systems;
          };
        };
      });
    };
}
