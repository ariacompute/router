import 'package:test/test.dart';
import 'package:aria_router/ariarouter.dart';

void main() {
  test('setup memory', () {
    final st = applySetup(SetupConfig(), baseUrl: 'http://127.0.0.1:8899', token: 't');
    expect(st.token, 't');
  });
}
