/// aria-router Flutter SDK (C ABI via FFI on device; auth is in-memory).
library;

class AuthConfig {
  String baseUrl;
  String token;
  AuthConfig({this.baseUrl = '', this.token = ''});
}

AuthConfig applyAuth(AuthConfig existing, {String? baseUrl, String? token}) {
  return AuthConfig(
    baseUrl: baseUrl ?? existing.baseUrl,
    token: token ?? existing.token,
  );
}
