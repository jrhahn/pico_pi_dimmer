{
  description = "Pico dimmer v4: Pico W as an MQTT light for Home Assistant";

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
      in
      {
        # `cargo run --release` builds and copies the image onto a Pico W in
        # BOOTSEL mode.
        devShells.default = pkgs.mkShell {
          packages = [ rustToolchain pkgs.elf2uf2-rs ];
        };
      });
}
