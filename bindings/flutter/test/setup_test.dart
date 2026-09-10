import 'dart:io';
import 'package:test/test.dart';
import 'package:aria_router/aria_router.dart';

void main() {
  test('setup memory', () {
    final st =
        applySetup(SetupConfig(), baseUrl: 'http://127.0.0.1:8899', token: 't');
    expect(st.token, 't');
  });

  final ffi = Platform.environment['ARIA_ROUTER_FFI_LIB'];
  final cfg = Platform.environment['ARIA_ROUTER_CONFIG'];
  final hasFfi = ffi != null &&
      ffi.isNotEmpty &&
      cfg != null &&
      cfg.isNotEmpty &&
      File(ffi).existsSync();

  test('init models complete', () {
    if (!hasFfi) {
      // Host CI skips when FFI env unset.
      return;
    }
    final r = AriaRouter()..init(cfg);
    try {
      final models = r.models();
      expect(models.toString().contains('semantic-auto'), isTrue);
      final out = r.complete(
        [
          {'role': 'user', 'content': 'hi'}
        ],
        {'model': 'ariacompute/semantic-auto'},
      );
      expect(out.toString().contains('hello-from-router'), isTrue);
      final lr = r.lastRoute();
      expect(lr['layer'], 'semantic');
    } finally {
      r.close();
    }
  }, skip: hasFfi ? false : 'ARIA_ROUTER_FFI_LIB / ARIA_ROUTER_CONFIG unset');

  test('init missing path', () {
    if (!hasFfi) return;
    expect(() => AriaRouter().init('/no/such.yaml'), throwsStateError);
  }, skip: hasFfi ? false : 'ARIA_ROUTER_FFI_LIB unset');

  test('connect without server', () {
    if (!hasFfi) return;
    final r = AriaRouter()..connect('http://127.0.0.1:9');
    r.close();
  }, skip: hasFfi ? false : 'ARIA_ROUTER_FFI_LIB unset');
}
