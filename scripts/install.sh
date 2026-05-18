#!/bin/sh
set -eu

REPO="${ROSWIRE_REPO:-AS153929/roswire}"
VERSION="${ROSWIRE_VERSION:-latest}"
INSTALL_DIR="${ROSWIRE_INSTALL_DIR:-${BINDIR:-/usr/local/bin}}"
VERIFY="${ROSWIRE_VERIFY:-1}"

usage() {
  cat <<'EOF'
Install roswire from GitHub Releases.

Usage:
  curl -fsSL https://raw.githubusercontent.com/AS153929/roswire/main/scripts/install.sh | sh

Environment:
  ROSWIRE_VERSION       Release tag to install, for example v0.0.3. Defaults to latest.
  ROSWIRE_INSTALL_DIR   Directory to install roswire into. Defaults to /usr/local/bin.
  ROSWIRE_REPO          GitHub repository. Defaults to AS153929/roswire.
  ROSWIRE_VERIFY=0      Skip SHA256 verification. Not recommended.
EOF
}

log() {
  printf '%s\n' "$*"
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required"
}

download() {
  url="$1"
  out="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$out"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$out" "$url"
  else
    die "curl or wget is required"
  fi
}

detect_asset() {
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux)
      case "$arch" in
        x86_64|amd64)
          printf '%s\n' "roswire-linux-amd64.tar.gz"
          ;;
        aarch64|arm64)
          printf '%s\n' "roswire-linux-arm64.tar.gz"
          ;;
        *)
          die "unsupported Linux architecture: $arch"
          ;;
      esac
      ;;
    Darwin)
      die "macOS prebuilt artifacts are not published yet; install from source with: cargo install --git https://github.com/$REPO --locked"
      ;;
    *)
      die "unsupported OS for this installer: $os"
      ;;
  esac
}

checksum_line() {
  checksums="$1"
  asset="$2"
  awk -v file="$asset" '$2 == file { print; found = 1 } END { if (!found) exit 1 }' "$checksums"
}

checksum_hash() {
  checksums="$1"
  asset="$2"
  awk -v file="$asset" '$2 == file { print $1; found = 1 } END { if (!found) exit 1 }' "$checksums"
}

verify_checksum() {
  checksums="$1"
  archive="$2"
  asset="$3"

  [ "$VERIFY" = "0" ] && {
    log "Skipping checksum verification because ROSWIRE_VERIFY=0."
    return
  }

  if command -v sha256sum >/dev/null 2>&1; then
    checksum_line "$checksums" "$asset" > "$TMPDIR_ROSWIRE/checksums.asset.txt" ||
      die "checksums.txt does not contain $asset"
    (cd "$TMPDIR_ROSWIRE" && sha256sum -c checksums.asset.txt) >/dev/null
  elif command -v shasum >/dev/null 2>&1; then
    expected="$(checksum_hash "$checksums" "$asset")" ||
      die "checksums.txt does not contain $asset"
    actual="$(shasum -a 256 "$archive" | awk '{ print $1 }')"
    [ "$expected" = "$actual" ] || die "checksum mismatch for $asset"
  else
    die "sha256sum or shasum is required for checksum verification"
  fi

  log "Verified SHA256 checksum."
}

install_binary() {
  src="$1"
  dst_dir="$2"
  dst="$dst_dir/roswire"

  if mkdir -p "$dst_dir" 2>/dev/null && install -m 0755 "$src" "$dst" 2>/dev/null; then
    log "Installed roswire to $dst"
    return
  fi

  if command -v sudo >/dev/null 2>&1; then
    sudo install -d "$dst_dir"
    sudo install -m 0755 "$src" "$dst"
    log "Installed roswire to $dst"
    return
  fi

  die "cannot write to $dst_dir; rerun with ROSWIRE_INSTALL_DIR set to a writable PATH directory"
}

case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
  "")
    ;;
  *)
    die "unknown argument: $1"
    ;;
esac

case "$VERSION" in
  latest)
    BASE_URL="https://github.com/$REPO/releases/latest/download"
    ;;
  v*)
    BASE_URL="https://github.com/$REPO/releases/download/$VERSION"
    ;;
  *)
    VERSION="v$VERSION"
    BASE_URL="https://github.com/$REPO/releases/download/$VERSION"
    ;;
esac

ASSET="$(detect_asset)"
TMPDIR_ROSWIRE="$(mktemp -d "${TMPDIR:-/tmp}/roswire-install.XXXXXX")"
trap 'rm -rf "$TMPDIR_ROSWIRE"' EXIT HUP INT TERM

ARCHIVE="$TMPDIR_ROSWIRE/$ASSET"
CHECKSUMS="$TMPDIR_ROSWIRE/checksums.txt"

log "Installing roswire from $REPO ($VERSION)."
log "Downloading $ASSET..."
download "$BASE_URL/$ASSET" "$ARCHIVE"

if [ "$VERIFY" = "0" ]; then
  log "Skipping checksum verification because ROSWIRE_VERIFY=0."
else
  download "$BASE_URL/checksums.txt" "$CHECKSUMS"
  verify_checksum "$CHECKSUMS" "$ARCHIVE" "$ASSET"
fi

case "$ASSET" in
  *.tar.gz)
    need_cmd tar
    tar -xzf "$ARCHIVE" -C "$TMPDIR_ROSWIRE"
    ;;
  *)
    die "unsupported archive format: $ASSET"
    ;;
esac

BIN="$TMPDIR_ROSWIRE/roswire"
[ -f "$BIN" ] || die "archive did not contain roswire binary"
chmod +x "$BIN"

install_binary "$BIN" "$INSTALL_DIR"

if [ -x "$INSTALL_DIR/roswire" ]; then
  "$INSTALL_DIR/roswire" --version
fi
