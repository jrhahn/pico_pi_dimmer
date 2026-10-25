{
  description = "Pico dimmer v3: Pico + Pimoroni Wireless Pack as an MQTT light for Home Assistant";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { nixpkgs, fenix, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        # rust-toolchain.toml stays the single source of truth for the Rust
        # version and the thumbv6m target, so plain rustup users get the same.
        rustToolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-gh/xTkxKHL4eiRXzWv8KP7vfjSk61Iq48x47BEDFgfk=";
        };

        # CadQuery is not in nixpkgs, so the CAD shell installs the pinned
        # wheels into a venv on first entry. They are manylinux builds and need
        # to be told where the system libraries live.
        cadPython = pkgs.python311;
        cadLibs = pkgs.lib.makeLibraryPath (with pkgs; [
          stdenv.cc.cc.lib
          expat
          zlib
          libGL
          glib
          fontconfig
          freetype
          libx11
          libxext
          libxrender
        ]);
        cadVenv = ".venv-cad";
      in
      {
        # Firmware: `cargo run --release` builds and copies the image onto a
        # Pico in BOOTSEL mode.
        devShells.default = pkgs.mkShell {
          packages = [ rustToolchain pkgs.elf2uf2-rs ];
        };

        # Enclosure:
        #   nix develop .#cad
        #   python cad/enclosure.py      # writes cad/export/*.stl and *.step
        devShells.cad = pkgs.mkShell {
          packages = [ cadPython ];
          LD_LIBRARY_PATH = cadLibs;
          shellHook = ''
            if [ ! -x "${cadVenv}/bin/python" ]; then
              echo "creating ${cadVenv} (one-off, pulls ~500 MB of OpenCASCADE)..."
              ${cadPython}/bin/python -m venv "${cadVenv}"
              "${cadVenv}/bin/pip" install --quiet --upgrade pip
              # Pinned: CadQuery moves its API around between minor releases.
              "${cadVenv}/bin/pip" install --quiet \
                'cadquery==2.8.0' 'cadquery-ocp==7.9.3.1.1' 'numpy>=2.0'
            fi
            export PATH="$PWD/${cadVenv}/bin:$PATH"
          '';
        };
      });
}
