#!/usr/bin/env bash
# Host binding tests for ariarouter: build libariarouter_ffi, run Rust/Python/Go/TS/RN.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "== cargo test ariacompute-ariarouter-ffi / ariacompute-ariarouter =="
cargo test -p ariacompute-ariarouter-ffi -p ariacompute-ariarouter

echo "== prepare FFI lib =="
cargo build -q -p ariacompute-ariarouter-ffi
CFG="$ROOT/config/examples/ffi-tiny.yaml"
export ARIAROUTER_CONFIG="$CFG"
export ARIA_INCLUDE="$ROOT/ffi/include"
if [[ "$(uname)" == "Darwin" ]]; then
  export ARIAROUTER_FFI_LIB="$ROOT/target/debug/libariarouter_ffi.dylib"
elif [[ "$(uname -s)" == MINGW* || "$(uname -s)" == MSYS* ]]; then
  export ARIAROUTER_FFI_LIB="$ROOT/target/debug/ariarouter_ffi.dll"
else
  export ARIAROUTER_FFI_LIB="$ROOT/target/debug/libariarouter_ffi.so"
fi
export LD_LIBRARY_PATH="${ROOT}/target/debug:${LD_LIBRARY_PATH:-}"
export DYLD_LIBRARY_PATH="${ROOT}/target/debug:${DYLD_LIBRARY_PATH:-}"
echo "ARIAROUTER_FFI_LIB=$ARIAROUTER_FFI_LIB"
test -e "$ARIAROUTER_FFI_LIB"

if command -v python3 >/dev/null; then
  echo "== python =="
  (cd bindings/python && PYTHONPATH=. python3 -m unittest discover -s tests -t . -v)
fi

if command -v go >/dev/null; then
  echo "== go =="
  (cd bindings/go && CGO_ENABLED=1 go test -tags ariarouter_ffi ./...)
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
