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
          config = {
            allowUnfree = true;
            android_sdk.accept_license = true;
          };
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

        developmentRustToolchain = rustToolchain.override {
          targets = [
            "aarch64-linux-android"
            "armv7-linux-androideabi"
            "x86_64-linux-android"
          ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            "aarch64-apple-ios"
            "aarch64-apple-ios-sim"
            "x86_64-apple-ios"
          ];
        };

        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        workspacePackage = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package;

        androidSdk = (pkgs.androidenv.composeAndroidPackages {
          platformVersions = [ "36" ];
          buildToolsVersions = [ "36.0.0" ];
          includeNDK = true;
          ndkVersion = "28.2.13676358";
        }).androidsdk;

        androidJdk = pkgs.jdk17;

        hostXcrun = pkgs.writeShellScriptBin "xcrun" ''
          exec /usr/bin/xcrun "$@"
        '';

        # Common dependencies across all platforms
        commonDeps = with pkgs; [
          developmentRustToolchain

          # tools
          androidJdk
          androidSdk
          cargo-expand
          cargo-ndk
          flutter
          flutter_rust_bridge_codegen
          git-cliff
          jujutsu
          hugo

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
            # Flutter Linux desktop build deps
            gtk3
            ninja

            # GUI deps (iced / wgpu)
            libxkbcommon
            wayland
            libx11
            libxcursor
            libxrandr
            libxi
            vulkan-loader
            vulkan-headers

            # Optional EPUB Wry spike (GTK 3 and WebKitGTK 4.1 development files)
            webkitgtk_4_1

            # Headless desktop E2E and screenshot verification
            xvfb
            xwininfo
            xdotool
            imagemagick
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

        macosDevDeps =
          with pkgs;
          pkgs.lib.optionals pkgs.stdenv.isDarwin [
            cocoapods
            hostXcrun
            # Flutter 3.41 relies on GNU rsync's --chmod before in-place lipo.
            rsync
          ];

        # Windows-specific dependencies (when cross-compiling or running on Windows)
        windowsDeps =
          with pkgs;
          pkgs.lib.optionals pkgs.stdenv.hostPlatform.isWindows [
            # Windows system APIs are automatically available
            # iced uses DirectX/DXGI which are built into Windows
            # No additional dependencies needed for native Windows builds
          ];

        packageRuntimeDeps =
          with pkgs;
          [ pdfium-binaries ]
          ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
            libglvnd
            libxkbcommon
            mesa
            wayland
            libx11
            libxcursor
            libxrandr
            libxi
            vulkan-loader
          ];

        shosai = rustPlatform.buildRustPackage (
          {
            pname = "shosai";
            inherit (workspacePackage) version;
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [
                ./Cargo.lock
                ./Cargo.toml
                ./assets/fonts
                ./assets/shosai-dev-icon.png
                ./assets/shosai-icon.png
                ./crates
                ./packaging/linux/shosai.desktop
              ];
            };

            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [
              "--package"
              "shosai-app"
              "--bin"
              "shosai"
            ];

            nativeBuildInputs = with pkgs; [
              clang
              cmake
              makeWrapper
              pkg-config
            ];

            buildInputs = (with pkgs; [ openssl ]) ++ packageRuntimeDeps ++ macosDeps;

            postInstall =
              pkgs.lib.optionalString pkgs.stdenv.isLinux ''
                wrapProgram "$out/bin/shosai" \
                  --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath packageRuntimeDeps} \
                  --set-default __EGL_VENDOR_LIBRARY_FILENAMES \
                    "${pkgs.mesa}/share/glvnd/egl_vendor.d/50_mesa.json" \
                  --set-default LIBGL_DRIVERS_PATH "${pkgs.mesa}/lib/dri"

                install -Dm644 packaging/linux/shosai.desktop \
                  "$out/share/applications/shosai.desktop"
                install -Dm644 assets/shosai-icon.png \
                  "$out/share/icons/hicolor/1024x1024/apps/shosai.png"
                substituteInPlace "$out/share/applications/shosai.desktop" \
                  --replace-fail '@SHOSAI_EXEC@' "$out/bin/shosai"
              ''
              + pkgs.lib.optionalString pkgs.stdenv.isDarwin ''
                wrapProgram "$out/bin/shosai" \
                  --prefix DYLD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath packageRuntimeDeps}
              ''
              + ''
                install -Dm644 assets/fonts/LICENSE-Inter \
                  "$out/share/licenses/shosai/INTER-LICENSE"
              '';

            meta = {
              description = "Native desktop ebook reader for PDF, EPUB, and CBZ files";
              homepage = workspacePackage.repository;
              license = with pkgs.lib.licenses; [ asl20 ofl ];
              mainProgram = "shosai";
              platforms = pkgs.pdfium-binaries.meta.platforms;
            };
          }
          // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath packageRuntimeDeps;
          }
          // pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin {
            DYLD_LIBRARY_PATH = pkgs.lib.makeLibraryPath packageRuntimeDeps;
          }
        );
      in
      {
        packages = {
          inherit shosai;
          default = shosai;
        };

        apps =
          let
            app = {
              type = "app";
              program = "${shosai}/bin/shosai";
              meta.description = "Run the Shosai ebook reader";
            };
          in
          {
            shosai = app;
            default = app;
          };

        checks.shosai = shosai;

        devShells.default = pkgs.mkShell (
          {
            nativeBuildInputs = commonDeps ++ linuxDeps ++ macosDeps ++ macosDevDeps ++ windowsDeps;

            # Common environment variables
            ANDROID_HOME = "${androidSdk}/libexec/android-sdk";
            ANDROID_SDK_ROOT = "${androidSdk}/libexec/android-sdk";
            ANDROID_NDK_HOME = "${androidSdk}/libexec/android-sdk/ndk-bundle";
            ANDROID_NDK_ROOT = "${androidSdk}/libexec/android-sdk/ndk-bundle";
            JAVA_HOME = androidJdk.home;
            GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${androidSdk}/libexec/android-sdk/build-tools/36.0.0/aapt2";
            RUST_SRC_PATH = "${developmentRustToolchain}/lib/rustlib/src/rust/library";
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
                webkitgtk_4_1
              ])
            );

            # WebKitGTK's propagated build environment can exceed GCC collect2's
            # spawn limit; Clang links the same native Cargo targets directly.
            CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = "clang";
            CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER = "clang";
          })
          // (pkgs.lib.optionalAttrs pkgs.stdenv.isDarwin {
            # macOS: DYLD_LIBRARY_PATH for dynamic libraries
            DYLD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.pdfium-binaries ];

            # Release artifacts must use the host Apple SDK and system libraries,
            # rather than embedding dependencies from the Nix SDK or store.
            CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER = "/usr/bin/clang";
            CC_aarch64_apple_darwin = "/usr/bin/clang";
            CXX_aarch64_apple_darwin = "/usr/bin/clang++";
            shellHook = ''
              export DEVELOPER_DIR="$(env -u DEVELOPER_DIR /usr/bin/xcode-select --print-path)"
              export SDKROOT="$(/usr/bin/xcrun --sdk macosx --show-sdk-path)"
              export PATH="${hostXcrun}/bin:$PATH"
              export MACOSX_DEPLOYMENT_TARGET=13.0
            '';
          })
        );
      }
    );
}
