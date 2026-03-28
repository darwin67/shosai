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

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfree = true;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustToolchain = pkgs.rust-bin.stable."1.94.0".default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
            "clippy"
            "rustfmt"
          ];
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
        linuxDeps =
          with pkgs;
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

        # macOS-specific dependencies
        macosDeps =
          with pkgs;
          pkgs.lib.optionals pkgs.stdenv.isDarwin [
            # Essential macOS libraries for Rust compilation
            libiconv
            # macOS system frameworks are automatically available
            # iced uses Metal and native APIs which are built into macOS
          ];

        # Windows-specific dependencies (when cross-compiling or running on Windows)
        windowsDeps =
          with pkgs;
          pkgs.lib.optionals pkgs.stdenv.hostPlatform.isWindows [
            # Windows system APIs are automatically available
            # iced uses DirectX/DXGI which are built into Windows
            # No additional dependencies needed for native Windows builds
          ];
      in
      {
        devShells.default = pkgs.mkShell (
          {
            nativeBuildInputs = commonDeps ++ linuxDeps ++ macosDeps ++ windowsDeps;

            # Common environment variables
            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          }
          // (pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            # Linux: LD_LIBRARY_PATH for shared libraries
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (
              [ pkgs.pdfium-binaries ]
              ++ (with pkgs; [
                libxkbcommon
                wayland
                libx11
                libxcursor
                libxrandr
                libxi
                vulkan-loader
              ])
            );
          })
          // (pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin {
            # macOS: DYLD_LIBRARY_PATH for dynamic libraries
            DYLD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.pdfium-binaries ];
          })
        );
      }
    );
}
