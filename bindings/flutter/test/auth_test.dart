import 'package:test/test.dart';
import 'package:aria_router/aria_router.dart';

void main() {
  test('auth memory', () {
    final st = applyAuth(AuthConfig(), baseUrl: 'http://127.0.0.1:8899', token: 't');
    expect(st.token, 't');
  });
}
