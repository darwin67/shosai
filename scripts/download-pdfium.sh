#!/usr/bin/env bash
# Download pre-built PDFium shared library from pdfium-binaries.
# Usage: ./scripts/download-pdfium.sh [output-dir]
#
# This downloads the appropriate binary for the current platform and
# extracts libpdfium into the specified directory (default: ./lib/).

set -euo pipefail

PDFIUM_VERSION="7749"
OUTPUT_DIR="${1:-./lib}"

detect_platform() {
	local os arch
	os="$(uname -s)"
	arch="$(uname -m)"

	case "$os" in
	Linux)
		case "$arch" in
		x86_64) echo "linux-x64" ;;
		aarch64) echo "linux-arm64" ;;
		*)
			echo "Unsupported Linux architecture: $arch" >&2
			exit 1
			;;
		esac
		;;
	Darwin)
		case "$arch" in
		x86_64) echo "mac-x64" ;;
		arm64) echo "mac-arm64" ;;
		*)
			echo "Unsupported macOS architecture: $arch" >&2
			exit 1
			;;
		esac
		;;
	*)
		echo "Unsupported OS: $os" >&2
		exit 1
		;;
	esac
}

PLATFORM="$(detect_platform)"
URL="https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/${PDFIUM_VERSION}/pdfium-${PLATFORM}.tgz"

echo "Platform: ${PLATFORM}"
echo "PDFium version: chromium/${PDFIUM_VERSION}"
echo "Download URL: ${URL}"
echo "Output directory: ${OUTPUT_DIR}"

mkdir -p "$OUTPUT_DIR"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo "Downloading PDFium..."
curl -sSL "$URL" -o "$TMPDIR/pdfium.tgz"

echo "Extracting..."
tar xzf "$TMPDIR/pdfium.tgz" -C "$TMPDIR"

# Copy the shared library
case "$(uname -s)" in
Linux)
	cp "$TMPDIR/lib/libpdfium.so" "$OUTPUT_DIR/"
	echo "Installed: ${OUTPUT_DIR}/libpdfium.so"
	;;
Darwin)
	cp "$TMPDIR/lib/libpdfium.dylib" "$OUTPUT_DIR/"
	echo "Installed: ${OUTPUT_DIR}/libpdfium.dylib"
	;;
esac

echo "Done. PDFium library is ready in ${OUTPUT_DIR}/"
