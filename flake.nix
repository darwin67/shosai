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

        # Common dependencies across all platforms
        commonDeps = with pkgs; [
          rustToolchain

          # tools
          git-cliff

          # build deps
          pkg-config
          cmake
          clang

          # runtime deps
          openssl
          pdfium-binaries

          # LSP
          rust-analyzer
          nodePackages.yaml-language-server
        ];

        # Linux-specific dependencies
        linuxDeps = with pkgs;
          pkgs.lib.optionals pkgs.stdenv.isLinux [
            # GUI deps (iced / wgpu)
            libxkbcommon
            wayland
            libx11
            libxcursor
            libxrandr
            libxi
            vulkan-loader
            vulkan-headers
          ];
      in {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = commonDeps ++ linuxDeps;

          # Ensure iced + pdfium can find shared libs at runtime
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath ([ pkgs.pdfium-binaries ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux (with pkgs; [
              libxkbcommon
              wayland
              libx11
              libxcursor
              libxrandr
              libxi
              vulkan-loader
            ]));

          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
        };
      });
}
