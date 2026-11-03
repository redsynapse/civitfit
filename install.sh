#!/bin/sh
set -eu

REPO="${CIVITFIT_REPO:-redsynapse/civitfit}"
BIN="civitfit"
VERSION="${CIVITFIT_VERSION:-latest}"
INSTALL_DIR="${CIVITFIT_INSTALL_DIR:-}"

usage() {
  cat <<EOF
Install $BIN from GitHub Releases. No Rust toolchain required.

Usage: install.sh [options]

Options:
  --local             Install to ~/.local/bin
  --dir DIR           Install to DIR
  --version VERSION   Install a specific release tag (default: latest)
  -h, --help          Show this help

Environment:
  CIVITFIT_INSTALL_DIR  Install directory
  CIVITFIT_VERSION      Release tag
  CIVITFIT_REPO         owner/repo (default: $REPO)
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --local)
      INSTALL_DIR="$HOME/.local/bin"
      shift
      ;;
    --dir)
      [ $# -ge 2 ] || { echo "error: --dir needs a value" >&2; exit 1; }
      INSTALL_DIR="$2"
      shift 2
      ;;
    --version)
      [ $# -ge 2 ] || { echo "error: --version needs a value" >&2; exit 1; }
      VERSION="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

command -v curl >/dev/null 2>&1 || {
  echo "error: curl is required" >&2
  exit 1
}

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Darwin) os_part="apple-darwin" ;;
  Linux) os_part="unknown-linux-gnu" ;;
  *)
    echo "error: unsupported OS: $os" >&2
    echo "Windows users: download the .zip from https://github.com/$REPO/releases" >&2
    exit 1
    ;;
esac

case "$arch" in
  arm64|aarch64) arch_part="aarch64" ;;
  x86_64|amd64) arch_part="x86_64" ;;
  *)
    echo "error: unsupported architecture: $arch" >&2
    exit 1
    ;;
esac

target="${arch_part}-${os_part}"

case "$VERSION" in
  [0-9]*) VERSION="v$VERSION" ;;
esac

if [ "$VERSION" = "latest" ]; then
  VERSION="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
    "https://github.com/$REPO/releases/latest" | sed 's|.*/||')"
  case "$VERSION" in
    v[0-9]*) ;;
    *)
      echo "error: could not find a published release for $REPO" >&2
      exit 1
      ;;
  esac
fi

asset="${BIN}-${VERSION}-${target}.tar.gz"
url="https://github.com/$REPO/releases/download/$VERSION/$asset"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "Downloading $asset"
curl -fsSL "$url" -o "$tmp/$asset"

if curl -fsSL "$url.sha256" -o "$tmp/$asset.sha256" 2>/dev/null; then
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$tmp" && sha256sum -c "$asset.sha256")
  else
    (cd "$tmp" && shasum -a 256 -c "$asset.sha256")
  fi
else
  echo "warning: checksum not available, skipping verification" >&2
fi

tar -xzf "$tmp/$asset" -C "$tmp"
src="$tmp/${BIN}-${VERSION}-${target}/$BIN"
[ -f "$src" ] || {
  echo "error: $BIN was not found in the archive" >&2
  exit 1
}

if [ -z "$INSTALL_DIR" ]; then
  if [ -w /usr/local/bin ] 2>/dev/null; then
    INSTALL_DIR="/usr/local/bin"
  else
    INSTALL_DIR="$HOME/.local/bin"
  fi
fi

mkdir -p "$INSTALL_DIR"
install -m 0755 "$src" "$INSTALL_DIR/$BIN"

echo "Installed $BIN $VERSION to $INSTALL_DIR/$BIN"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "Add it to PATH: export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
"$INSTALL_DIR/$BIN" --version
