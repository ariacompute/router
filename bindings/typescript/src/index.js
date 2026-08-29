export class Router {
  constructor() {
    this._auth = { base_url: "", token: "" };
  }

  auth(u = {}) {
    if (u.base_url !== undefined) this._auth.base_url = u.base_url;
    if (u.token !== undefined) this._auth.token = u.token;
    return this;
  }

  authStatus() {
    return { ...this._auth };
  }

  authClear() {
    this._auth = { base_url: "", token: "" };
    return this;
  }
}
