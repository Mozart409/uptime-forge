{
  description = "Development environment for a Node.js project";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-25.05";
    unstable.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs = {
    self,
    nixpkgs,
    unstable,
    flake-utils,
    rust-overlay,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
        config.allowUnfree = true;
        overlays = [rust-overlay.overlays.default];
      };
      rust = pkgs.rust-bin.stable."1.92.0".default;
    in {
      # to use other shells, run:
      # nix develop . --command fish
      devShells.default = pkgs.mkShell {
        buildInputs = with pkgs; [
          rust
          lazydocker
          bacon
          cargo-deny
          lefthook
          cocogitto
          just
          cargo-workspaces
          opentofu
          dbeaver-bin
          postgresql_16
        ];
      };
    });
}
