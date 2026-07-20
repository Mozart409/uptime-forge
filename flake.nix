{
  description = "Development environment for a Node.js project";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    rust-overlay,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        config.allowUnfree = true;
        overlays = [rust-overlay.overlays.default];
      };
      rust = pkgs.rust-bin.stable."1.96.1".default;
    in {
      # to use other shells, run:
      # nix develop . --command fish
      devShells.default = pkgs.mkShell {
        buildInputs = with pkgs; [
          # keep-sorted start
          cargo-deny
          cargo-watch
          cargo-workspaces
          cocogitto
          dbeaver-bin
          just
          keep-sorted
          lazydocker
          lefthook
          opencode
          opentofu
          podman
          podman-compose
          postgresql_16
          rust
          sqlx-cli
          tailwindcss_4
          # keep-sorted end
        ];
        shellHook = ''
          lefthook install
          du -sh ./target/
        '';
      };
    });
}
