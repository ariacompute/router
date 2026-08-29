#!/usr/bin/env bash
# Host binding tests for aria-router: build libaria_router_ffi, run Rust/Python/Go/TS/RN.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "== cargo test ariacompute-router-ffi / ariacompute-router =="
cargo test -p ariacompute-router-ffi -p ariacompute-router

echo "== prepare FFI lib =="
cargo build -q -p ariacompute-router-ffi
CFG="$ROOT/config/examples/ffi-tiny.yaml"
export ARIA_ROUTER_CONFIG="$CFG"
export ARIA_INCLUDE="$ROOT/ffi/include"
if [[ "$(uname)" == "Darwin" ]]; then
  export ARIA_ROUTER_FFI_LIB="$ROOT/target/debug/libaria_router_ffi.dylib"
elif [[ "$(uname -s)" == MINGW* || "$(uname -s)" == MSYS* ]]; then
  export ARIA_ROUTER_FFI_LIB="$ROOT/target/debug/aria_router_ffi.dll"
else
  export ARIA_ROUTER_FFI_LIB="$ROOT/target/debug/libaria_router_ffi.so"
fi
export LD_LIBRARY_PATH="${ROOT}/target/debug:${LD_LIBRARY_PATH:-}"
export DYLD_LIBRARY_PATH="${ROOT}/target/debug:${DYLD_LIBRARY_PATH:-}"
echo "ARIA_ROUTER_FFI_LIB=$ARIA_ROUTER_FFI_LIB"
test -e "$ARIA_ROUTER_FFI_LIB"

if command -v python3 >/dev/null; then
  echo "== python =="
  (cd bindings/python && PYTHONPATH=. python3 -m unittest discover -s tests -t . -v)
fi

if command -v go >/dev/null; then
  echo "== go =="
  (cd bindings/go && CGO_ENABLED=1 go test -tags aria_router_ffi ./...)
fi

if command -v node >/dev/null && [[ -f bindings/typescript/package.json ]]; then
  echo "== typescript =="
  (cd bindings/typescript && node --test test/binding.test.mjs)
fi

if command -v node >/dev/null && [[ -f bindings/react-native/package.json ]]; then
  echo "== react-native setup =="
  (cd bindings/react-native && node --test test/setup.test.cjs)
fi

echo "done (mobile Swift/Kotlin/Flutter: see bindings/*/ )"
