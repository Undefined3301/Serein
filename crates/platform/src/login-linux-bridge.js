// GTK4/WebKit6 login only. Tokens stay in this main-frame closure until a bounded
// native main-frame evaluation reads them; cross-frame IPC carries only a wake bit.
(() => {
  if (window !== window.top || location.origin !== "https://discord.com") return;
  const capability = "__SEREIN_LOGIN_CAPABILITY__";
  const opened = Date.now();
  let pending = null;
  let closed = false;
  const invalidate = () => {
    closed = true;
    pending = null;
  };
  const active = () => {
    if (window !== window.top || location.origin !== "https://discord.com" || Date.now() - opened > 600000) invalidate();
    return !closed;
  };
  window.addEventListener("pagehide", invalidate, { once: true });
  Object.defineProperty(window, "__serein_login_peek_" + capability.slice(0, -1), {
    // Evaluation or native acceptance may fail; only view teardown/expiry/navigation clears it.
    value: () => active() ? pending : null,
    writable: false,
    configurable: false,
  });
  Object.defineProperty(window, "ipc", {
    value: Object.freeze({ postMessage(value) {
      if (!active() || pending !== null || typeof value !== "string" || value.length > 2113 || !value.startsWith(capability)) return false;
      const token = value.slice(capability.length);
      if (token.length < 16 || token.length > 2048 || !/^[\x21-\x7e]+$/.test(token)) return false;
      pending = value;
      // Wakes are optional: native also polls the protected main-frame slot.
      try { window.webkit.messageHandlers.sereinLogin.postMessage(true); } catch {}
      return true;
    }}),
    writable: false,
    configurable: false,
  });
})();
