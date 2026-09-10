#!/usr/bin/env bash
# Host binding tests for aria-router: build libaria_router_ffi (cdylib), run Rust/Python/Go/TS/RN/Flutter/Kotlin.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "== cargo test ariacompute-router-ffi / ariacompute-router =="
cargo test -p ariacompute-router-ffi -p ariacompute-router

echo "== prepare FFI lib =="
cargo build -q -p ariacompute-router-ffi
CFG="$ROOT/bindings/testdata/fast-response.yaml"
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
  if [[ ! -d bindings/typescript/node_modules ]]; then
    npm install --prefix bindings/typescript
  fi
  (cd bindings/typescript && node --test test/binding.test.mjs)
fi

if command -v node >/dev/null && [[ -f bindings/react-native/package.json ]]; then
  echo "== react-native =="
  if [[ ! -d bindings/react-native/node_modules ]]; then
    npm install --prefix bindings/react-native
  fi
  (cd bindings/react-native && node --test test/setup.test.cjs)
fi

if command -v dart >/dev/null && [[ -f bindings/flutter/pubspec.yaml ]]; then
  echo "== flutter =="
  (cd bindings/flutter && dart pub get && dart test)
else
  echo "== flutter skipped (dart not found) =="
fi

GRADLE_BIN=""
if command -v gradle >/dev/null; then
  GRADLE_BIN="$(command -v gradle)"
elif [[ -x "$ROOT/bindings/kotlin/gradlew" ]]; then
  GRADLE_BIN="$ROOT/bindings/kotlin/gradlew"
fi
if [[ -n "$GRADLE_BIN" && -f bindings/kotlin/build.gradle ]]; then
  echo "== kotlin =="
  (cd bindings/kotlin && "$GRADLE_BIN" test)
else
  echo "== kotlin skipped (gradle/java not found) =="
fi

echo "done (Swift: bindings/swift Package.swift + host dlopen; run swift test when toolchain available)"
