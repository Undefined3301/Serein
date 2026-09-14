// Identity hints for the Chrome user-agent used by this temporary login window.
// No Chromium APIs are emulated. Namespace detection may select unsupported page
// code; live compatibility must be checked before treating this as a login fix.
(() => {
  if (window !== window.top || location.origin !== "https://discord.com") return;
  try {
    Object.defineProperty(navigator, "vendor", {
      get: () => "Google Inc.", configurable: true, enumerable: true,
    });
  } catch {}
  try {
    if (typeof window.chrome === "undefined") {
      Object.defineProperty(window, "chrome", {
        value: {}, configurable: true, enumerable: true, writable: true,
      });
    }
  } catch {}
})();
