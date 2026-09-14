// Offline only: synthetic response bodies and numeric diagnostic IPC.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('crates/platform/src/login-qr-diagnostics.js', 'utf8');
const endpoint = 'https://discord.com/api/v9/users/@me/remote-auth/login';
const flush = () => new Promise(resolve => setImmediate(resolve));
function setup({ origin = 'https://discord.com', frame = false, fetchError = null, openError = null } = {}) {
  const messages = [], calls = [], timers = new Map(), events = {};
  let now = 0, nextTimer = 0, fetchResult = Promise.resolve(null);
  const openResult = {};
  class XHR {
    constructor() { this.listeners = {}; this.status = 400; this.responseType = ''; this.responseURL = endpoint; this.text = '{"code":50035}'; this.reads = 0; }
    open(...args) { calls.push({ receiver: this, args }); if (openError) throw openError; return openResult; }
    addEventListener(name, fn) { this.listeners[name] = fn; }
    get responseText() { this.reads++; return this.text; }
    finish() { this.listeners.loadend?.(); }
  }
  const context = {
    navigator: {}, location: { origin, href: origin + '/login' }, XMLHttpRequest: XHR,
    URL, Request, Uint8Array, TextDecoder, Date: { now: () => now },
    webkit: { messageHandlers: { sereinQrDiagnostics: { postMessage: value => messages.push(value) } } },
    addEventListener: (name, fn) => { events[name] = fn; },
    setTimeout: (fn, delay) => { assert.ok(delay > 0 && delay <= 5000); timers.set(++nextTimer, fn); return nextTimer; },
    clearTimeout: id => timers.delete(id),
    fetch(...args) { calls.push({ receiver: this, args }); if (fetchError) throw fetchError; return fetchResult; },
  };
  context.window = context; context.top = frame ? {} : context;
  vm.runInNewContext(source, context);
  return { context, messages, calls, timers, openResult,
    response: value => fetchResult = Promise.resolve(value),
    result: value => fetchResult = value,
    expire: () => { now = 600000; }, close: () => events.pagehide?.(),
  };
}
function response(text = '{"code":50035}', status = 400, url = endpoint) {
  const result = { status, url, clones: 0, reads: 0, cancels: 0 };
  result.clone = () => {
    result.clones++;
    let done = false;
    return { body: { getReader: () => ({
      read() { result.reads++; if (done) return Promise.resolve({ done: true }); done = true; return Promise.resolve({ done: false, value: new TextEncoder().encode(text) }); },
      cancel() { result.cancels++; return Promise.resolve(); },
    }) } };
  };
  return result;
}
(async () => {
  {
    const s = setup(), r = response();
    const promise = s.response(r), receiver = {}, init = { get headers() { throw Error('must not inspect headers'); } };
    assert.equal(s.context.fetch.call(receiver, endpoint, init, 'extra'), promise);
    assert.deepEqual(s.calls[0], { receiver, args: [endpoint, init, 'extra'] });
    await flush();
    assert.deepEqual(s.messages, [50035400]);
    assert.equal(r.clones, 1);
    assert.equal(s.timers.size, 0);
  }
  for (const status of [200, 204, 302, 399, 600]) {
    const s = setup(), r = response('secret success body', status);
    s.response(r); s.context.fetch(endpoint); await flush();
    assert.equal(r.clones, 0); assert.deepEqual(s.messages, []);
    const xhr = new s.context.XMLHttpRequest(); xhr.open('POST', endpoint); xhr.status = status; xhr.finish();
    assert.equal(xhr.reads, 0);
  }
  for (const text of ['x'.repeat(4097), 'bad json', '{"code":"50035"}', '{"code":-1}', '{"code":1.5}', '{"code":1000000000}', '{"nested":{"code":1}}', 'null', '[1]']) {
    const s = setup(), r = response(text);
    s.response(r); s.context.fetch(endpoint); await flush();
    const xhr = new s.context.XMLHttpRequest(); xhr.open('POST', endpoint); xhr.text = text; xhr.finish();
    assert.deepEqual(s.messages, []);
  }
  for (const code of [0, 999999999]) {
    const s = setup(), xhr = new s.context.XMLHttpRequest();
    assert.equal(xhr.open('POST', endpoint, true, 'unused'), s.openResult);
    xhr.text = JSON.stringify({ code }); xhr.status = 599; xhr.finish(); xhr.finish();
    assert.deepEqual(s.messages, [code * 1000 + 599]);
  }
  for (const url of ['https://evil.test/api/v9/users/@me/remote-auth/login', 'http://discord.com/api/v9/users/@me/remote-auth/login', endpoint + '/extra', 'https://user@discord.com/api/v9/users/@me/remote-auth/login']) {
    const s = setup(), r = response(); s.response(r); s.context.fetch(url); await flush();
    assert.equal(r.clones, 0);
    const xhr = new s.context.XMLHttpRequest(); xhr.open('POST', url); xhr.finish(); assert.equal(xhr.reads, 0);
  }
  for (const options of [{ origin: 'https://evil.test' }, { frame: true }]) {
    const s = setup(options), r = response(); s.response(r); s.context.fetch(endpoint); await flush(); assert.equal(r.clones, 0);
  }
  {
    const error = Error('original');
    const s = setup({ fetchError: error, openError: error });
    assert.throws(() => s.context.fetch(endpoint), value => value === error);
    assert.throws(() => new s.context.XMLHttpRequest().open('POST', endpoint), value => value === error);
    const rejected = Promise.reject(error), other = setup(); other.result(rejected);
    assert.equal(other.context.fetch(endpoint), rejected); await assert.rejects(rejected, value => value === error); await flush();
  }
  {
    const s = setup(), r = response(); r.clone = () => { throw Error('clone unavailable'); };
    s.response(r); s.context.fetch(endpoint); await flush();
    s.response(response()); s.context.fetch(endpoint); await flush(); assert.deepEqual(s.messages, [50035400]);
    s.context.webkit.messageHandlers.sereinQrDiagnostics.postMessage = () => { throw Error('bridge unavailable'); };
    s.response(response()); s.context.fetch(endpoint); await flush();
  }
  for (const stop of ['timeout', 'close', 'expire', 'origin']) {
    const s = setup(), r = response(); let finish;
    r.clone = () => { r.clones++; return { body: { getReader: () => ({ read: () => new Promise(resolve => { finish = resolve; }), cancel: () => { r.cancels++; return Promise.resolve(); } }) } }; };
    s.response(r); s.context.fetch(endpoint); await flush();
    const other = response(); s.response(other); s.context.fetch(endpoint); await flush(); assert.equal(other.clones, 0);
    if (stop === 'timeout') [...s.timers.values()].forEach(fn => fn());
    if (stop === 'close') s.close();
    if (stop === 'expire') s.expire();
    if (stop === 'origin') s.context.location.origin = 'https://evil.test';
    finish({ done: true }); await flush();
    assert.deepEqual(s.messages, []); assert.ok(r.cancels > 0); assert.equal(s.timers.size, 0);
  }
  {
    const s = setup();
    for (let n = 0; n < 70; n++) { const xhr = new s.context.XMLHttpRequest(); xhr.open('POST', endpoint); xhr.finish(); }
    assert.equal(s.messages.length, 64);
    const r = response(); s.response(r); s.context.fetch(endpoint); await flush(); assert.equal(r.clones, 0);
  }
  {
    const s = setup(), xhr = new s.context.XMLHttpRequest(); xhr.open('POST', endpoint); xhr.responseType = 'json'; xhr.finish(); assert.equal(xhr.reads, 0);
    const r = response('{"code":1}', 400, 'https://evil.test/'); s.response(r); s.context.fetch(endpoint); await flush(); assert.equal(r.clones, 0);
  }
  console.log('QR diagnostics: bounded error-only numeric IPC, request preservation, scope, cancellation and attempt limits passed (synthetic only).');
})().catch(error => { console.error(error); process.exitCode = 1; });
