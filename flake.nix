{
  description = "Shōsai (書斎) — Ebook Reader";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfree = true;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustToolchain = pkgs.rust-bin.stable."1.94.0".default.override {
          extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
        };
      in {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rustToolchain

            # tools
            git-cliff

            # build deps
            pkg-config
            cmake
            clang

            # runtime deps
            openssl

            # GUI deps (iced / wgpu)
            libxkbcommon
            wayland
            xorg.libX11
            xorg.libXcursor
            xorg.libXrandr
            xorg.libXi
            vulkan-loader
            vulkan-headers

            # LSP
            rust-analyzer
            nodePackages.yaml-language-server
          ];

          # Ensure iced can find GPU drivers and Wayland/X11 libs at runtime
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
            libxkbcommon
            wayland
            xorg.libX11
            xorg.libXcursor
            xorg.libXrandr
            xorg.libXi
            vulkan-loader
          ]);

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
        };
      });
}
