// Runs only in Serein's newly-created ephemeral authentication webview.
// Observe its own same-origin API Authorization header after the owner logs in.
// Discord still performs the entire authentication/challenge/QR flow.
(() => {
  if (window !== window.top || location.origin !== "https://discord.com") return;
  const capability = "__SEREIN_LOGIN_CAPABILITY__";
  let delivered = false;
  const opened = Date.now();
  const allowed = value => {
    try {
      const url = new URL(value, location.href);
      return url.origin === "https://discord.com" && /^\/api\/v\d+\//.test(url.pathname);
    } catch { return false; }
  };
  const deliver = value => {
    if (delivered || window !== window.top || location.origin !== "https://discord.com" || Date.now() - opened > 600000 || typeof value !== "string" || value.length < 16 || value.length > 2048 || !/^[\x21-\x7e]+$/.test(value)) return;
    const bridge = window.ipc;
    if (typeof bridge?.postMessage !== "function") return;
    // Linux reports whether its bounded slot stored the candidate. Other native bridges
    // return undefined on success. A missing/throwing/rejecting bridge may be retried.
    if (bridge.postMessage(capability + value) !== false) delivered = true;
  };
  const destinations = new WeakMap();
  const open = XMLHttpRequest.prototype.open;
  const setHeader = XMLHttpRequest.prototype.setRequestHeader;
  XMLHttpRequest.prototype.open = function() {
    const result = open.apply(this, arguments);
    try { destinations.set(this, allowed(arguments[1])); } catch {}
    return result;
  };
  XMLHttpRequest.prototype.setRequestHeader = function(name, value) {
    const result = setHeader.apply(this, arguments);
    try {
      if (destinations.get(this) && typeof name === "string" && name.toLowerCase() === "authorization") deliver(value);
    } catch {}
    return result;
  };
  const originalFetch = window.fetch;
  window.fetch = function(input, init) {
    const result = originalFetch.apply(this, arguments);
    try {
      if (allowed(input instanceof Request ? input.url : input)) {
        const headers = new Headers(init?.headers ?? (input instanceof Request ? input.headers : undefined));
        deliver(headers.get("authorization"));
      }
    } catch {}
    return result;
  };
})();
