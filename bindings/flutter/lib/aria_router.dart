/// aria-router Flutter SDK (C ABI via dart:ffi; auth is in-memory).
library;

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'package:ffi/ffi.dart';

class SetupConfig {
  String baseUrl;
  String token;
  SetupConfig({this.baseUrl = '', this.token = ''});
}

SetupConfig applySetup(SetupConfig existing, {String? baseUrl, String? token}) {
  return SetupConfig(
    baseUrl: baseUrl ?? existing.baseUrl,
    token: token ?? existing.token,
  );
}

String _ariaHome() {
  final override = Platform.environment['ARIA_COMPUTE_HOME'];
  if (override != null && override.isNotEmpty) return override;
  final home = Platform.isWindows
      ? Platform.environment['USERPROFILE']
      : Platform.environment['HOME'];
  return '${home ?? '.'}/.ariacompute';
}

List<String> _ffiLibNames() {
  if (Platform.isWindows) {
    return ['aria-router_ffi.dll', 'aria_router_ffi.dll'];
  }
  if (Platform.isMacOS || Platform.isIOS) {
    return ['libaria-router_ffi.dylib', 'libaria_router_ffi.dylib'];
  }
  return ['libaria-router_ffi.so', 'libaria_router_ffi.so'];
}

String? _firstExisting(String dir) {
  for (final name in _ffiLibNames()) {
    final p = '$dir/$name';
    if (File(p).existsSync()) return p;
  }
  return null;
}

String _resolveLibPath([String? explicit]) {
  if (explicit != null && File(explicit).existsSync()) return explicit;
  final env = Platform.environment['ARIA_ROUTER_FFI_LIB'];
  if (env != null && env.isNotEmpty && File(env).existsSync()) return env;
  final cached = _firstExisting('${_ariaHome()}/lib');
  if (cached != null) return cached;
  throw StateError('libaria-router_ffi not found; set ARIA_ROUTER_FFI_LIB');
}

String _expandHome(String p) {
  if (p == '~') {
    return Platform.environment['HOME'] ??
        Platform.environment['USERPROFILE'] ??
        p;
  }
  if (p.startsWith('~/')) {
    final home = Platform.environment['HOME'] ??
        Platform.environment['USERPROFILE'] ??
        '';
    return '$home/${p.substring(2)}';
  }
  return p;
}

typedef _InitC = Pointer Function(Pointer<Utf8>);
typedef _InitDart = Pointer Function(Pointer<Utf8>);
typedef _ConnectC = Pointer Function(Pointer<Utf8>);
typedef _ConnectDart = Pointer Function(Pointer<Utf8>);
typedef _DestroyC = Void Function(Pointer);
typedef _DestroyDart = void Function(Pointer);
typedef _SetupC = Void Function(Pointer, Pointer<Utf8>, Pointer<Utf8>);
typedef _SetupDart = void Function(Pointer, Pointer<Utf8>, Pointer<Utf8>);
typedef _CompleteC = Int32 Function(
    Pointer, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, IntPtr);
typedef _CompleteDart = int Function(
    Pointer, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, int);
typedef _BufOutC = Int32 Function(Pointer, Pointer<Utf8>, IntPtr);
typedef _BufOutDart = int Function(Pointer, Pointer<Utf8>, int);
typedef _LastErrorC = Pointer<Utf8> Function();
typedef _LastErrorDart = Pointer<Utf8> Function();

class AriaRouter {
  DynamicLibrary? _lib;
  Pointer? _handle;
  SetupConfig _auth = SetupConfig();
  String? _libPath;

  late final _InitDart _init;
  late final _ConnectDart _connect;
  late final _DestroyDart _destroy;
  late final _SetupDart _setup;
  late final _CompleteDart _complete;
  late final _BufOutDart _models;
  late final _BufOutDart _lastRoute;
  late final _LastErrorDart _lastError;

  AriaRouter({String? libPath}) {
    _libPath = libPath;
  }

  AriaRouter setup({String? baseUrl, String? token}) {
    _auth = applySetup(_auth, baseUrl: baseUrl, token: token);
    final handle = _handle;
    if (handle != null && handle.address != 0) {
      _syncFfiSetup(baseUrl: baseUrl, token: token);
    }
    return this;
  }

  SetupConfig setupStatus() =>
      SetupConfig(baseUrl: _auth.baseUrl, token: _auth.token);

  AriaRouter setupClear() {
    _auth = SetupConfig();
    final handle = _handle;
    if (handle != null && handle.address != 0) {
      _syncFfiSetup(baseUrl: '', token: '');
    }
    return this;
  }

  void _ensure() {
    if (_lib != null) return;
    final path = _resolveLibPath(_libPath);
    _lib = DynamicLibrary.open(path);
    _init = _lib!.lookupFunction<_InitC, _InitDart>('aria_router_init');
    _connect =
        _lib!.lookupFunction<_ConnectC, _ConnectDart>('aria_router_connect');
    _destroy =
        _lib!.lookupFunction<_DestroyC, _DestroyDart>('aria_router_destroy');
    _setup = _lib!.lookupFunction<_SetupC, _SetupDart>('aria_router_setup');
    _complete =
        _lib!.lookupFunction<_CompleteC, _CompleteDart>('aria_router_complete');
    _models = _lib!.lookupFunction<_BufOutC, _BufOutDart>('aria_router_models');
    _lastRoute =
        _lib!.lookupFunction<_BufOutC, _BufOutDart>('aria_router_last_route');
    _lastError =
        _lib!.lookupFunction<_LastErrorC, _LastErrorDart>('aria_router_last_error');
  }

  void _syncFfiSetup({String? baseUrl, String? token}) {
    final handle = _handle;
    if (handle == null || handle.address == 0) return;
    final bu = baseUrl != null ? baseUrl.toNativeUtf8() : nullptr;
    final tok = token != null ? token.toNativeUtf8() : nullptr;
    try {
      _setup(handle, bu, tok);
    } finally {
      if (bu.address != 0) malloc.free(bu);
      if (tok.address != 0) malloc.free(tok);
    }
  }

  void _syncAuthIfSet() {
    if (_auth.baseUrl.isNotEmpty || _auth.token.isNotEmpty) {
      _syncFfiSetup(
        baseUrl: _auth.baseUrl.isNotEmpty ? _auth.baseUrl : null,
        token: _auth.token.isNotEmpty ? _auth.token : null,
      );
    }
  }

  String _err(String fallback) {
    final p = _lastError();
    if (p.address == 0) return fallback;
    final s = p.toDartString();
    return s.isEmpty ? fallback : s;
  }

  AriaRouter init([String? configPath]) {
    _ensure();
    if (_handle != null) close();
    Pointer<Utf8> pathPtr = nullptr;
    if (configPath != null && configPath.trim().isNotEmpty) {
      pathPtr = _expandHome(configPath.trim()).toNativeUtf8();
    }
    try {
      _handle = _init(pathPtr);
    } finally {
      if (pathPtr.address != 0) malloc.free(pathPtr);
    }
    if (_handle == null || _handle!.address == 0) {
      throw StateError(_err('init failed'));
    }
    _syncAuthIfSet();
    return this;
  }

  AriaRouter connect(String baseUrl) {
    _ensure();
    if (_handle != null) close();
    final url = baseUrl.toNativeUtf8();
    try {
      _handle = _connect(url);
    } finally {
      malloc.free(url);
    }
    if (_handle == null || _handle!.address == 0) {
      throw StateError(_err('connect failed'));
    }
    _syncAuthIfSet();
    return this;
  }

  void close() {
    final lib = _lib;
    final handle = _handle;
    if (lib != null && handle != null && handle.address != 0) {
      _destroy(handle);
    }
    _handle = null;
  }

  void destroy() => close();

  Map<String, dynamic> complete(List<dynamic> messages,
      [Map<String, dynamic>? options]) {
    final handle = _handle;
    if (handle == null || handle.address == 0) {
      throw StateError('router not initialized');
    }
    final out = malloc<Uint8>(256 * 1024).cast<Utf8>();
    final m = jsonEncode(messages).toNativeUtf8();
    final o = jsonEncode(options ?? <String, dynamic>{}).toNativeUtf8();
    try {
      final rc = _complete(handle, m, o, out, 256 * 1024);
      if (rc != 0) throw StateError(_err('complete failed'));
      return jsonDecode(out.toDartString()) as Map<String, dynamic>;
    } finally {
      malloc.free(m);
      malloc.free(o);
      malloc.free(out);
    }
  }

  Map<String, dynamic> models() {
    final handle = _handle;
    if (handle == null || handle.address == 0) {
      throw StateError('router not initialized');
    }
    final out = malloc<Uint8>(64 * 1024).cast<Utf8>();
    try {
      final rc = _models(handle, out, 64 * 1024);
      if (rc != 0) throw StateError(_err('models failed'));
      return jsonDecode(out.toDartString()) as Map<String, dynamic>;
    } finally {
      malloc.free(out);
    }
  }

  Map<String, dynamic> lastRoute() {
    final handle = _handle;
    if (handle == null || handle.address == 0) return {};
    final out = malloc<Uint8>(64 * 1024).cast<Utf8>();
    try {
      final rc = _lastRoute(handle, out, 64 * 1024);
      if (rc != 0) return {};
      final s = out.toDartString();
      if (s.isEmpty) return {};
      return jsonDecode(s) as Map<String, dynamic>;
    } catch (_) {
      return {};
    } finally {
      malloc.free(out);
    }
  }
}
