export class Router {
  constructor() {
    this._auth = { base_url: "", token: "" };
  }

  setup(u = {}) {
    if (u.base_url !== undefined) this._auth.base_url = u.base_url;
    if (u.token !== undefined) this._auth.token = u.token;
    return this;
  }

  setupStatus() {
    return { ...this._auth };
  }

  setupClear() {
    this._auth = { base_url: "", token: "" };
    return this;
  }
}
