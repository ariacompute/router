"""Aria Router Python binding (ctypes over libaria_router_ffi)."""
from __future__ import annotations

import ctypes
import json
import os
import sys
from ctypes import c_char_p, c_int, c_size_t, c_void_p
from typing import Any, Optional

__version__ = "0.1.0"

_LIB_NAMES = {
    "win32": "aria_router_ffi.dll",
    "darwin": "libaria_router_ffi.dylib",
}


def _ffi_lib_name() -> str:
    return _LIB_NAMES.get(sys.platform, "libaria_router_ffi.so")


def _aria_home() -> str:
    return os.environ.get("ARIA_COMPUTE_HOME") or os.path.join(os.path.expanduser("~"), ".ariacompute")


def _load_lib(path: Optional[str] = None):
    if not path:
        path = os.environ.get("ARIA_ROUTER_FFI_LIB")
    if not path:
        bundled = os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", _ffi_lib_name())
        path = bundled if os.path.isfile(bundled) else None
    if not path:
        cached = os.path.join(_aria_home(), "lib", _ffi_lib_name())
        path = cached if os.path.isfile(cached) else None
    if not path:
        raise RuntimeError("libaria_router_ffi not found; set ARIA_ROUTER_FFI_LIB")
    return ctypes.CDLL(path)


class Router:
    def __init__(self, lib=None):
        self._lib = lib
        self._handle = None
        self._auth = {"base_url": "", "token": ""}

    def setup(self, base_url: Optional[str] = None, token: Optional[str] = None) -> "Router":
        if base_url is not None:
            self._auth["base_url"] = base_url
        if token is not None:
            self._auth["token"] = token
        return self

    def setup_status(self) -> dict[str, str]:
        return dict(self._auth)

    def setup_clear(self) -> "Router":
        self._auth = {"base_url": "", "token": ""}
        return self

    def _ensure(self) -> None:
        if self._lib is not None:
            return
        self._lib = _load_lib()
        self._lib.aria_router_init.restype = c_void_p
        self._lib.aria_router_init.argtypes = [c_char_p]
        self._lib.aria_router_connect.restype = c_void_p
        self._lib.aria_router_connect.argtypes = [c_char_p]
        self._lib.aria_router_destroy.argtypes = [c_void_p]
        self._lib.aria_router_complete.restype = c_int
        self._lib.aria_router_complete.argtypes = [c_void_p, c_char_p, c_char_p, c_char_p, c_size_t]
        self._lib.aria_router_complete_stream.restype = c_int
        self._lib.aria_router_models.restype = c_int
        self._lib.aria_router_models.argtypes = [c_void_p, c_char_p, c_size_t]
        self._lib.aria_router_last_route.restype = c_int
        self._lib.aria_router_last_route.argtypes = [c_void_p, c_char_p, c_size_t]
        self._lib.aria_router_last_error.restype = c_char_p

    def init(self, config_path: str) -> "Router":
        self._ensure()
        if self._handle:
            self.close()
        self._handle = self._lib.aria_router_init(config_path.encode())
        if not self._handle:
            err = self._lib.aria_router_last_error()
            raise RuntimeError(err.decode() if err else "init failed")
        return self

    def connect(self, base_url: str) -> "Router":
        self._ensure()
        if self._handle:
            self.close()
        self._handle = self._lib.aria_router_connect(base_url.encode())
        if not self._handle:
            err = self._lib.aria_router_last_error()
            raise RuntimeError(err.decode() if err else "connect failed")
        return self

    def close(self) -> None:
        if self._handle and self._lib:
            self._lib.aria_router_destroy(self._handle)
        self._handle = None

    def complete(self, messages: list, options: Optional[dict[str, Any]] = None) -> dict:
        if not self._handle:
            raise RuntimeError("router not initialized")
        buf = ctypes.create_string_buffer(256 * 1024)
        rc = self._lib.aria_router_complete(
            self._handle,
            json.dumps(messages).encode(),
            json.dumps(options or {}).encode(),
            buf,
            len(buf),
        )
        if rc != 0:
            err = self._lib.aria_router_last_error()
            raise RuntimeError(err.decode() if err else "complete failed")
        return json.loads(buf.value.decode())

    def models(self) -> dict:
        if not self._handle:
            raise RuntimeError("router not initialized")
        buf = ctypes.create_string_buffer(64 * 1024)
        rc = self._lib.aria_router_models(self._handle, buf, len(buf))
        if rc != 0:
            err = self._lib.aria_router_last_error()
            raise RuntimeError(err.decode() if err else "models failed")
        return json.loads(buf.value.decode())

    def last_route(self) -> dict:
        if not self._handle:
            return {}
        buf = ctypes.create_string_buffer(64 * 1024)
        rc = self._lib.aria_router_last_route(self._handle, buf, len(buf))
        if rc != 0:
            return {}
        return json.loads(buf.value.decode() or "{}")
