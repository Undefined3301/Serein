// Observe only bounded error codes from this window's QR ticket exchange.
// Never change requests, read successful bodies, or send response text to native code.
(() => {
  if (window !== window.top || location.origin !== "https://discord.com") return;
  const opened = Date.now();
  let closed = false, attempts = 0, reading = false, cancelRead = null;
  const invalidate = () => { closed = true; cancelRead?.(); };
  const active = () => {
    if (window !== window.top || location.origin !== "https://discord.com" || Date.now() - opened >= 600000) invalidate();
    return !closed;
  };
  window.addEventListener("pagehide", invalidate, { once: true });
  const allowed = value => {
    if (typeof value !== "string" || value.length > 2048) return false;
    const url = new URL(value, location.href);
    return url.origin === "https://discord.com" && !url.username && !url.password && /^\/api\/v\d+\/users\/@me\/remote-auth\/login$/.test(url.pathname);
  };
  const errorStatus = status => Number.isInteger(status) && status >= 400 && status <= 599;
  const report = (text, status) => {
    if (!active() || !errorStatus(status) || typeof text !== "string" || text.length > 4096) return;
    const body = JSON.parse(text);
    if (body === null || typeof body !== "object" || Array.isArray(body) || !Object.prototype.hasOwnProperty.call(body, "code")) return;
    const code = body.code;
    if (Number.isInteger(code) && code >= 0 && code <= 999999999) {
      window.webkit.messageHandlers.sereinQrDiagnostics.postMessage(code * 1000 + status);
    }
  };
  const observe = async response => {
    let reader = null, timer = null, ownsRead = false;
    try {
      if (!active() || reading || !errorStatus(response.status) || !allowed(response.url)) return;
      reading = true;
      ownsRead = true;
      reader = response.clone().body.getReader();
      let stopped = false;
      const timeout = new Promise(resolve => {
        cancelRead = () => {
          stopped = true;
          try { Promise.resolve(reader.cancel()).catch(() => {}); } catch {}
          resolve({ done: true });
        };
        timer = setTimeout(cancelRead, Math.min(5000, 600000 - (Date.now() - opened)));
      });
      const bytes = new Uint8Array(4096);
      let length = 0;
      while (active() && !stopped) {
        const chunk = await Promise.race([reader.read(), timeout]);
        if (stopped || !active()) return;
        if (chunk.done) {
          report(new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(0, length)), response.status);
          return;
        }
        if (!(chunk.value instanceof Uint8Array) || chunk.value.byteLength > bytes.length - length) return;
        bytes.set(chunk.value, length);
        length += chunk.value.byteLength;
      }
    } catch {} finally {
      if (reader !== null) {
        try { Promise.resolve(reader.cancel()).catch(() => {}); } catch {}
      }
      if (timer !== null) clearTimeout(timer);
      if (ownsRead) { cancelRead = null; reading = false; }
    }
  };
  const fetch = window.fetch;
  window.fetch = function(input) {
    const result = fetch.apply(this, arguments);
    try {
      if (active() && attempts < 64 && allowed(input instanceof Request ? input.url : input)) {
        attempts++;
        result.then(observe, () => {}).catch(() => {});
      }
    } catch {}
    return result;
  };
  const destinations = new WeakMap();
  const open = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function() {
    const result = open.apply(this, arguments);
    try {
      let state = destinations.get(this);
      if (state) state.selected = false;
      if (active() && attempts < 64 && allowed(arguments[1])) {
        attempts++;
        if (!state) {
          state = { selected: false };
          destinations.set(this, state);
          this.addEventListener("loadend", () => {
            try {
              if (!state.selected || !active()) return;
              state.selected = false;
              if (!errorStatus(this.status) || !allowed(this.responseURL) || !["", "text"].includes(this.responseType)) return;
              const text = this.responseText;
              if (typeof text === "string" && text.length <= 4096) report(text, this.status);
            } catch {}
          });
        }
        state.selected = true;
      }
    } catch {}
    return result;
  };
})();
