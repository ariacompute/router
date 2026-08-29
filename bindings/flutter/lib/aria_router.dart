/// aria-router Flutter SDK (C ABI via FFI on device; auth is in-memory).
library;

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
