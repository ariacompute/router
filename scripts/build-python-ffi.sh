#!/usr/bin/env bash
# Build libaria_router_ffi (cdylib) and copy as libaria-router_ffi into the Python wheel.
# Used as cibuildwheel CIBW_BEFORE_ALL so each platform wheel bundles its own lib.
#
# Context: on Linux this runs INSIDE a manylinux container (no Rust preinstalled);
# on macOS/Windows it runs on the runner host (Rust preinstalled by the workflow).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1; then
  echo "==> Installing rustup (minimal profile)..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
  fi
  export PATH="$HOME/.cargo/bin:$PATH"
  command -v cargo >/dev/null 2>&1 || {
    echo "ERROR: cargo still not on PATH after rustup install" >&2
    exit 1
  }
fi

echo "==> cargo $(cargo --version) (CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-<default>})"

HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
case "$HOST_TRIPLE" in
  *-musl*)
    echo "ERROR: rust host triple is '$HOST_TRIPLE' (musl)." >&2
    echo "Rust drops the cdylib crate type on musl targets (crt-static default)," >&2
    echo "so libaria-router_ffi.so cannot be produced here. musllinux wheels are" >&2
    echo "disabled; make sure CIBW_SKIP=musllinux* is applied so only manylinux" >&2
    echo "builds run." >&2
    exit 1
    ;;
esac

case "$(uname -s)" in
  Darwin)          LIB="libaria_router_ffi.dylib"; DEST_NAME="libaria-router_ffi.dylib";;
  MINGW*|MSYS*|CYGWIN*) LIB="aria_router_ffi.dll"; DEST_NAME="aria-router_ffi.dll";;
  *)               LIB="libaria_router_ffi.so"; DEST_NAME="libaria-router_ffi.so";;
esac

cargo build --release -p ariacompute-router-ffi

SRC=""
for c in "target/release/$LIB" "${CARGO_TARGET_DIR:+$CARGO_TARGET_DIR/release/$LIB}"; do
  if [ -n "$c" ] && [ -f "$c" ]; then
    SRC="$c"
    break
  fi
done
if [ -z "$SRC" ]; then
  SRC="$(find . -maxdepth 4 -type f -name "$LIB" -path '*/release/*' 2>/dev/null | head -1 || true)"
fi
if [ -z "$SRC" ]; then
  echo "ERROR: $LIB not found after 'cargo build --release -p ariacompute-router-ffi'" >&2
  echo "--- target/release ---" >&2
  ls -la target/release 2>/dev/null | head -20 || true
  echo "--- found libs ---" >&2
  find . -maxdepth 4 \( -name '*.so' -o -name '*.dylib' -o -name '*.dll' \) 2>/dev/null | head -20 || true
  exit 1
fi

DEST="bindings/python/aria_router/lib"
mkdir -p "$DEST"
cp "$SRC" "$DEST/$DEST_NAME"
echo "FFI copied $SRC -> $DEST/$DEST_NAME"
