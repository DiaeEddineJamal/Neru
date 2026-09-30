#!/usr/bin/env bash
# Installs the `neru` terminal CLI on macOS and Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/DiaeEddineJamal/Neru/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/DiaeEddineJamal/Neru/main/install.sh | bash -s -- 0.4.0
#
# Installs into ${NERU_HOME:-$HOME/.neru}/cli and links ~/.local/bin/neru to it.
# Set NERU_VERSION instead of passing an argument to pin a version, and NERU_NO_MODIFY_PATH=1 to
# leave shell profiles alone.
set -eu

REPO="DiaeEddineJamal/Neru"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != "dumb" ]; then
  MOSS="$(printf '\033[38;5;107m')"
  DIM="$(printf '\033[2m')"
  BOLD="$(printf '\033[1m')"
  RED="$(printf '\033[31m')"
  RESET="$(printf '\033[0m')"
else
  MOSS="" DIM="" BOLD="" RED="" RESET=""
fi

say() { printf '%s\n' "$*"; }
step() { printf '  %s%s%s\n' "$DIM" "$*" "$RESET"; }
fail() {
  printf '%serror:%s %s\n' "$RED" "$RESET" "$*" >&2
  exit 1
}

VERSION="${1:-${NERU_VERSION:-}}"
VERSION="${VERSION#v}"
NERU_HOME="${NERU_HOME:-$HOME/.neru}"
INSTALL_DIR="$NERU_HOME/cli"
BIN_DIR="$HOME/.local/bin"

printf '\n%s✻ Neru%s\n\n' "$MOSS$BOLD" "$RESET"

# Platform
os="$(uname -s)"
arch="$(uname -m)"
case "$arch" in
  arm64 | aarch64) arch="aarch64" ;;
  x86_64 | amd64) arch="x86_64" ;;
  *) fail "unsupported architecture: $arch" ;;
esac
case "$os" in
  Darwin) target="$arch-apple-darwin" ;;
  Linux)
    if [ "$arch" != "x86_64" ]; then
      fail "Linux $arch is not built yet. Build from source: https://github.com/$REPO#build-from-source"
    fi
    target="x86_64-unknown-linux-gnu"
    ;;
  *) fail "unsupported system: $os. On Windows, run: irm https://raw.githubusercontent.com/$REPO/main/install.ps1 | iex" ;;
esac

asset="neru-cli-$target.tar.gz"
if [ -n "${NERU_DOWNLOAD_BASE:-}" ]; then
  # Mirrors and local testing: a folder URL that holds the asset and its .sha256.
  base="${NERU_DOWNLOAD_BASE%/}"
elif [ -n "$VERSION" ]; then
  base="https://github.com/$REPO/releases/download/v$VERSION"
else
  base="https://github.com/$REPO/releases/latest/download"
fi

# Tools
if command -v curl >/dev/null 2>&1; then
  download() { curl -fsSL --retry 3 -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
  download() { wget -q -O "$2" "$1"; }
else
  fail "curl or wget is required"
fi
if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | awk '{print $1}'; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | awk '{print $1}'; }
else
  fail "sha256sum or shasum is required to verify the download"
fi
command -v tar >/dev/null 2>&1 || fail "tar is required"

tmp="$(mktemp -d 2>/dev/null || mktemp -d -t neru)"
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT INT TERM

# Download and verify
step "Downloading $asset${VERSION:+ (v$VERSION)}"
download "$base/$asset" "$tmp/$asset" || fail "could not download $base/$asset"
download "$base/$asset.sha256" "$tmp/$asset.sha256" || fail "could not download the checksum for $asset"

expected="$(awk '{print $1; exit}' "$tmp/$asset.sha256" | tr 'A-F' 'a-f')"
actual="$(sha256 "$tmp/$asset")"
[ -n "$expected" ] || fail "the checksum file is empty"
[ "$expected" = "$actual" ] || fail "checksum mismatch for $asset (expected $expected, got $actual)"
step "Checksum verified"

# Extract beside the install dir, then swap it in
mkdir -p "$NERU_HOME"
staging="$NERU_HOME/.cli-new.$$"
rm -rf "$staging"
mkdir -p "$staging"
tar -xzf "$tmp/$asset" -C "$staging" || { rm -rf "$staging"; fail "could not extract $asset"; }
[ -f "$staging/neru" ] || { rm -rf "$staging"; fail "the archive does not contain neru"; }
chmod +x "$staging/neru"

if [ "$os" = "Darwin" ]; then
  xattr -dr com.apple.quarantine "$staging" 2>/dev/null || true
fi

old="$NERU_HOME/.cli-old.$$"
if [ -e "$INSTALL_DIR" ]; then
  mv "$INSTALL_DIR" "$old"
fi
if ! mv "$staging" "$INSTALL_DIR"; then
  [ -e "$old" ] && mv "$old" "$INSTALL_DIR"
  rm -rf "$staging"
  fail "could not install into $INSTALL_DIR"
fi
rm -rf "$old"
step "Installed to $INSTALL_DIR"

# Link onto PATH
mkdir -p "$BIN_DIR"
ln -sf "$INSTALL_DIR/neru" "$BIN_DIR/neru"
step "Linked $BIN_DIR/neru"

case ":${PATH:-}:" in
  *":$BIN_DIR:"*) on_path=1 ;;
  *) on_path=0 ;;
esac
if [ "$on_path" = 0 ] && [ "${NERU_NO_MODIFY_PATH:-}" = 1 ]; then
  say ""
  say "$BIN_DIR is not on your PATH; NERU_NO_MODIFY_PATH is set, so no profile was changed."
elif [ "$on_path" = 0 ]; then
  shell_name="$(basename "${SHELL:-sh}")"
  # The rc line is written literally so $HOME expands when the shell starts.
  # shellcheck disable=SC2016
  case "$shell_name" in
    zsh)
      rc="${ZDOTDIR:-$HOME}/.zshrc"
      line='export PATH="$HOME/.local/bin:$PATH"'
      ;;
    bash)
      rc="$HOME/.bashrc"
      line='export PATH="$HOME/.local/bin:$PATH"'
      ;;
    fish)
      rc="$HOME/.config/fish/config.fish"
      line='fish_add_path "$HOME/.local/bin"'
      ;;
    *)
      rc=""
      line='export PATH="$HOME/.local/bin:$PATH"'
      ;;
  esac
  say ""
  if [ -n "$rc" ]; then
    mkdir -p "$(dirname "$rc")"
    if [ ! -f "$rc" ] || ! grep -Fqx "$line" "$rc"; then
      printf '\n# Added by the Neru installer\n%s\n' "$line" >>"$rc"
      say "Added $BIN_DIR to PATH in $rc."
    fi
    say "Open a new terminal, or run: ${BOLD}$line${RESET}"
  else
    say "$BIN_DIR is not on your PATH. Add this to your shell profile:"
    say "  $line"
  fi
fi

# Linux: the CLI links against WebKitGTK like the desktop app
if [ "$os" = "Linux" ]; then
  have_webkit=0
  if command -v ldconfig >/dev/null 2>&1 && ldconfig -p 2>/dev/null | grep -q 'libwebkit2gtk-4\.1\.so'; then
    have_webkit=1
  else
    for dir in /usr/lib /usr/lib64 /usr/lib/x86_64-linux-gnu /usr/local/lib /lib/x86_64-linux-gnu; do
      if ls "$dir"/libwebkit2gtk-4.1.so* >/dev/null 2>&1; then
        have_webkit=1
        break
      fi
    done
  fi
  if [ "$have_webkit" = 0 ]; then
    say ""
    say "neru needs WebKitGTK 4.1, which was not found. Install it with one of:"
    if command -v apt-get >/dev/null 2>&1; then
      say "  sudo apt install libwebkit2gtk-4.1-0"
    elif command -v dnf >/dev/null 2>&1; then
      say "  sudo dnf install webkit2gtk4.1"
    elif command -v pacman >/dev/null 2>&1; then
      say "  sudo pacman -S webkit2gtk-4.1"
    else
      say "  Debian/Ubuntu: sudo apt install libwebkit2gtk-4.1-0"
      say "  Fedora:        sudo dnf install webkit2gtk4.1"
      say "  Arch:          sudo pacman -S webkit2gtk-4.1"
    fi
  fi
fi

say ""
say "${MOSS}✻${RESET} neru ${VERSION:+v$VERSION }is installed."
say "Run \`neru\` in any project folder."
say ""
