#!/bin/sh
# Starpod installer
# Usage: curl -fsSL https://raw.githubusercontent.com/sinaptik-ai/starpod/main/install.sh | sh
# Options:
#   --no-homebrew       Skip Homebrew even if available
#   --version=VERSION   Install a specific version (default: latest)
#   INSTALL_DIR=path    Override install location (default: ~/.local/bin)

set -eu

REPO="sinaptik-ai/starpod"
BINARY="starpod"
DEFAULT_INSTALL_DIR="$HOME/.local/bin"
INSTALL_DIR="${INSTALL_DIR:-$DEFAULT_INSTALL_DIR}"

# --- Colors (only if stdout is a terminal) ---

if [ -t 1 ]; then
    BOLD='\033[1m'
    DIM='\033[2m'
    GREEN='\033[32m'
    RED='\033[31m'
    YELLOW='\033[33m'
    RESET='\033[0m'
else
    BOLD='' DIM='' GREEN='' RED='' YELLOW='' RESET=''
fi

info()  { printf "${BOLD}${GREEN}==> %s${RESET}\n" "$*"; }
warn()  { printf "${BOLD}${YELLOW}warn:${RESET} %s\n" "$*"; }
error() { printf "${BOLD}${RED}error:${RESET} %s\n" "$*" >&2; }
die()   { error "$@"; exit 1; }

# --- Parse arguments ---

USE_HOMEBREW=true
VERSION=""

for arg in "$@"; do
    case "$arg" in
        --no-homebrew)    USE_HOMEBREW=false ;;
        --version=*)      VERSION="${arg#--version=}" ;;
        --help|-h)
            printf "Usage: curl -fsSL <url> | sh [-s -- OPTIONS]\n"
            printf "Options:\n"
            printf "  --no-homebrew       Skip Homebrew\n"
            printf "  --version=VERSION   Pin to a specific version\n"
            printf "  INSTALL_DIR=path    Override install location\n"
            exit 0
            ;;
        *) die "Unknown option: $arg" ;;
    esac
done

# --- Detect OS and architecture ---

detect_os() {
    case "$(uname -s)" in
        Darwin*)  echo "darwin" ;;
        Linux*)   echo "linux" ;;
        MINGW*|MSYS*|CYGWIN*)
            die "Windows is not supported by this installer. Download from https://github.com/$REPO/releases"
            ;;
        *) die "Unsupported OS: $(uname -s)" ;;
    esac
}

detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)   echo "x86_64" ;;
        arm64|aarch64)   echo "aarch64" ;;
        *) die "Unsupported architecture: $(uname -m)" ;;
    esac
}

OS="$(detect_os)"
ARCH="$(detect_arch)"

# --- Resolve version ---

resolve_version() {
    if [ -n "$VERSION" ]; then
        echo "$VERSION"
        return
    fi

    info "Fetching latest version..."
    LATEST=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
        | grep '"tag_name"' \
        | sed 's/.*"tag_name": *"//;s/".*//')

    if [ -z "$LATEST" ]; then
        die "Could not determine latest version. Set --version= manually."
    fi

    echo "$LATEST"
}

VERSION="$(resolve_version)"

# Strip leading 'v' for display but keep it for tag
DISPLAY_VERSION="${VERSION#v}"

info "Installing starpod $DISPLAY_VERSION for $OS/$ARCH"

# --- Homebrew path (macOS) ---

if [ "$OS" = "darwin" ] && [ "$USE_HOMEBREW" = true ] && command -v brew >/dev/null 2>&1; then
    info "Installing via Homebrew..."
    brew install sinaptik-ai/tap/starpod
    printf "\n${BOLD}${GREEN}starpod $DISPLAY_VERSION installed successfully!${RESET}\n"
    exit 0
fi

# --- Binary tarball install ---

# Build target triple
case "$OS" in
    darwin) TARGET="${ARCH}-apple-darwin" ;;
    linux)  TARGET="${ARCH}-unknown-linux-gnu" ;;
esac

TARBALL="starpod-${VERSION}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/$REPO/releases/download/${VERSION}/${TARBALL}"
CHECKSUM_URL="${DOWNLOAD_URL}.sha256"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# Download tarball
info "Downloading $TARBALL..."
curl -fSL --progress-bar -o "$TMPDIR/$TARBALL" "$DOWNLOAD_URL" \
    || die "Download failed. Check that version $VERSION exists at https://github.com/$REPO/releases"

# Download and verify checksum
info "Verifying checksum..."
curl -fsSL -o "$TMPDIR/$TARBALL.sha256" "$CHECKSUM_URL" \
    || die "Checksum download failed."

cd "$TMPDIR"
if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$TARBALL.sha256" >/dev/null 2>&1 \
        || die "Checksum verification failed!"
elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "$TARBALL.sha256" >/dev/null 2>&1 \
        || die "Checksum verification failed!"
else
    warn "No sha256sum or shasum found — skipping checksum verification"
fi

# Extract and install
info "Installing to $INSTALL_DIR..."
tar xzf "$TARBALL"
mkdir -p "$INSTALL_DIR"
mv "$BINARY" "$INSTALL_DIR/$BINARY"
chmod +x "$INSTALL_DIR/$BINARY"

# --- PATH instructions ---

check_path() {
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) return 0 ;;
        *) return 1 ;;
    esac
}

if ! check_path; then
    warn "$INSTALL_DIR is not in your PATH"
    printf "\n"

    SHELL_NAME="$(basename "${SHELL:-/bin/sh}")"
    case "$SHELL_NAME" in
        zsh)
            printf "  Add this to your ${BOLD}~/.zshrc${RESET}:\n"
            printf "    ${DIM}export PATH=\"%s:\$PATH\"${RESET}\n" "$INSTALL_DIR"
            ;;
        bash)
            printf "  Add this to your ${BOLD}~/.bashrc${RESET}:\n"
            printf "    ${DIM}export PATH=\"%s:\$PATH\"${RESET}\n" "$INSTALL_DIR"
            ;;
        fish)
            printf "  Run:\n"
            printf "    ${DIM}fish_add_path %s${RESET}\n" "$INSTALL_DIR"
            ;;
        *)
            printf "  Add ${BOLD}%s${RESET} to your PATH\n" "$INSTALL_DIR"
            ;;
    esac
    printf "\n"
fi

printf "${BOLD}${GREEN}starpod $DISPLAY_VERSION installed successfully!${RESET}\n"
printf "${DIM}Run 'starpod --help' to get started.${RESET}\n"
