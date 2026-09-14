// Offline VM test only. Node is never a messaging runtime dependency.
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const capability = 'a'.repeat(64) + ':';
const peekName = '__serein_login_peek_' + capability.slice(0, -1);
const handoff = fs.readFileSync('crates/platform/src/login-handoff.js', 'utf8')
  .replace('__SEREIN_LOGIN_CAPABILITY__', capability);
const linuxBridge = fs.readFileSync('crates/platform/src/login-linux-bridge.js', 'utf8')
  .replace('__SEREIN_LOGIN_CAPABILITY__', capability);
const marker = 'SYNTHETIC_SESSION_MARKER';
const duplicate = 'SYNTHETIC_DUPLICATE_MARKER';

function setup({ origin = 'https://discord.com', frame = false, linux = false, overrides = {} } = {}) {
  const messages = [];
  const calls = { open: [], header: [], fetch: [] };
  const results = { open: {}, header: {}, fetch: Promise.resolve() };
  const listeners = new Map();
  let now = 0;
  class XHR {
    open(...args) { calls.open.push({ receiver: this, args }); return results.open; }
    setRequestHeader(...args) { calls.header.push({ receiver: this, args }); return results.header; }
  }
  const context = {
    location: { origin, href: origin + '/login' }, XMLHttpRequest: XHR,
    Headers, Request, URL, WeakMap, Date: { now: () => now },
    addEventListener: (name, listener) => listeners.set(name, listener),
    webkit: { messageHandlers: { sereinLogin: { postMessage: value => messages.push(value) } } },
    fetch(...args) { calls.fetch.push({ receiver: this, args }); return results.fetch; },
    ...overrides,
  };
  context.window = context;
  context.top = frame ? {} : context;
  if (!linux && !Object.hasOwn(overrides, 'ipc')) {
    context.ipc = { postMessage: value => messages.push(value) };
  }
  vm.createContext(context);
  if (linux) vm.runInContext(linuxBridge, context);
  vm.runInContext(handoff, context);
  return { context, messages, calls, results, advance: elapsed => now = elapsed,
    peek: () => context[peekName](), pagehide: () => listeners.get('pagehide')?.() };
}

// Original requests run first and retain exact receivers, argument lists, return values and errors.
for (const linux of [false, true]) {
  const { context, messages, calls, results } = setup({ linux });
  const xhr = new context.XMLHttpRequest();
  const openArgs = ['GET', '/api/v10/users/@me', false, 'name', 'password'];
  assert.equal(xhr.open(...openArgs), results.open);
  assert.deepEqual(calls.open, [{ receiver: xhr, args: openArgs }]);
  assert.equal(xhr.setRequestHeader('Authorization', marker, 'extra'), results.header);
  assert.deepEqual(calls.header, [{ receiver: xhr, args: ['Authorization', marker, 'extra'] }]);
  const input = new Request('https://discord.com/api/v10/users/@me', { headers: { authorization: marker } });
  const init = { headers: { authorization: duplicate } };
  const receiver = {};
  assert.equal(context.fetch.call(receiver, input, init, 'extra'), results.fetch);
  assert.equal(calls.fetch[0].receiver, receiver);
  assert.deepEqual(calls.fetch[0].args, [input, init, 'extra']);
  assert.equal(input.headers.get('authorization'), marker);
  assert.equal(init.headers.authorization, duplicate);
  assert.deepEqual(messages, linux ? [true] : [capability + marker]);
}
{
  const originalError = new Error('synthetic original failure');
  class XHR {
    open() { throw originalError; }
    setRequestHeader() { throw originalError; }
  }
  const { context, messages } = setup({ overrides: {
    XMLHttpRequest: XHR, fetch() { throw originalError; },
  } });
  const xhr = new context.XMLHttpRequest();
  assert.throws(() => xhr.open('GET', '/api/v10/users/@me'), error => error === originalError);
  assert.throws(() => xhr.setRequestHeader('Authorization', marker), error => error === originalError);
  assert.throws(() => context.fetch('/api/v10/users/@me', { headers: { authorization: marker } }), error => error === originalError);
  assert.equal(messages.length, 0);
}
{
  const { context, calls } = setup();
  const xhr = new context.XMLHttpRequest();
  xhr.open(); xhr.setRequestHeader(); context.fetch();
  assert.equal(calls.open[0].args.length, 0);
  assert.equal(calls.header[0].args.length, 0);
  assert.equal(calls.fetch[0].args.length, 0);
}

// Observation failures never turn successful requests into failures or latch capture.
for (const bridge of [undefined, {}, { postMessage() { throw new Error('bridge unavailable'); } },
    { postMessage() { return false; } }]) {
  for (const transport of ['fetch', 'xhr']) {
    const { context, calls, messages, results } = setup({ overrides: { ipc: bridge } });
    const xhr = new context.XMLHttpRequest();
    xhr.open('GET', '/api/v10/users/@me');
    const send = value => transport === 'fetch'
      ? context.fetch('/api/v10/users/@me', { headers: { authorization: value } })
      : xhr.setRequestHeader('Authorization', value);
    assert.equal(send(marker), results[transport === 'fetch' ? 'fetch' : 'header']);
    context.ipc = { postMessage: value => messages.push(value) };
    send(duplicate);
    assert.deepEqual(messages, [capability + duplicate]);
    assert.equal(calls[transport === 'fetch' ? 'fetch' : 'header'].length, 2);
  }
}
for (const failure of ['headers', 'request', 'url', 'bridge-getter']) {
  const { context, calls, messages, results } = setup();
  const fail = () => { throw new Error('synthetic observer failure'); };
  const init = { headers: { authorization: marker } };
  if (failure === 'headers') context.Headers = class { constructor() { fail(); } };
  if (failure === 'request') context.Request = { [Symbol.hasInstance]: fail };
  if (failure === 'url') context.URL = class { constructor() { fail(); } };
  if (failure === 'bridge-getter') Object.defineProperty(context, 'ipc', { get: fail, configurable: true });
  assert.equal(context.fetch('/api/v10/users/@me', init), results.fetch);
  assert.equal(calls.fetch.length, 1);
  assert.equal(messages.length, 0);
}
{
  const { context, results, calls, messages } = setup();
  const init = Object.defineProperty({}, 'headers', { get() { throw new Error('headers read'); } });
  assert.equal(context.fetch('/api/v10/users/@me', init), results.fetch);
  assert.equal(calls.fetch[0].args[1], init);
  const xhr = new context.XMLHttpRequest();
  const badUrl = { toString() { throw new Error('URL read'); } };
  assert.equal(xhr.open('GET', badUrl), results.open);
  assert.equal(calls.open[0].args[1], badUrl);
  const badName = { toString() { throw new Error('header name read'); } };
  assert.equal(xhr.setRequestHeader(badName, marker), results.header);
  assert.equal(calls.header[0].args[0], badName);
  assert.equal(messages.length, 0);
}

for (const linux of [false, true]) {
  const { context, messages } = setup({ linux });
  const xhr = new context.XMLHttpRequest();
  for (const url of ['https://evil.test/api/v10/users/@me', 'https://discord.com.evil.test/api/v10/users/@me',
      'http://discord.com/api/v10/users/@me', '/not-api', '/api/v10']) {
    xhr.open('GET', url);
    xhr.setRequestHeader('Authorization', marker);
    context.fetch(url, { headers: { authorization: marker } });
  }
  xhr.open('GET', '/api/v10/users/@me');
  for (const value of [null, {}, 'short', 'x'.repeat(2049), ' '.repeat(16), '\u00e9'.repeat(16), '\x7f'.repeat(16)]) {
    xhr.setRequestHeader('Authorization', value);
  }
  assert.equal(messages.length, 0);
  context.fetch(new Request('https://discord.com/api/v10/users/@me', { headers: { authorization: marker } }));
  xhr.setRequestHeader('Authorization', duplicate);
  assert.deepEqual(messages, linux ? [true] : [capability + marker]);
}
for (const options of [{ origin: 'https://evil.test' }, { frame: true }]) {
  const { context, messages } = setup({ ...options, linux: true });
  assert.equal(context.ipc, undefined);
  assert.equal(context[peekName], undefined);
  context.fetch('https://discord.com/api/v10/users/@me', { headers: { authorization: marker } });
  assert.equal(messages.length, 0);
}

// Reads are not native acceptance. A failed evaluation may retry the protected candidate.
{
  const { context, messages, peek, pagehide } = setup({ linux: true });
  for (const value of [null, true, {}, 'wrong:' + marker, capability + 'x'.repeat(2049),
      capability + 'short', capability + ' '.repeat(16), capability + '\u00e9'.repeat(16)]) {
    assert.equal(context.ipc.postMessage(value), false);
  }
  assert.equal(peek(), null);
  const ipc = context.ipc;
  const originalPeek = context[peekName];
  vm.runInContext('window.ipc = {}; window.ipc.postMessage = () => {}; window[' + JSON.stringify(peekName) + '] = () => "wrong";', context);
  assert.equal(context.ipc, ipc);
  assert.equal(context[peekName], originalPeek);
  assert.equal(Object.isFrozen(ipc), true);
  assert.equal(Object.getOwnPropertyDescriptor(context, 'ipc').configurable, false);
  assert.equal(Object.getOwnPropertyDescriptor(context, peekName).writable, false);
  assert.equal(Object.getOwnPropertyDescriptor(context, peekName).configurable, false);
  context.fetch('/api/v10/users/@me', { headers: { authorization: marker } });
  assert.deepEqual(messages, [true]);
  assert.throws(() => vm.runInContext('window[' + JSON.stringify(peekName) + '](); throw new Error("synthetic evaluation failure")', context));
  assert.equal(peek(), capability + marker);
  assert.equal(peek(), capability + marker);
  assert.equal(context.ipc.postMessage(capability + duplicate), false);
  assert.equal(peek(), capability + marker);
  assert.deepEqual(messages, [true]);
  pagehide();
  assert.equal(peek(), null);
  assert.equal(context.ipc.postMessage(capability + duplicate), false);
}
for (const webkit of [undefined, {}, { messageHandlers: {} }, {
  messageHandlers: { sereinLogin: { postMessage() { throw new Error('wake unavailable'); } } },
}]) {
  const { context, messages, results, peek } = setup({ linux: true, overrides: { webkit } });
  assert.equal(context.fetch('/api/v10/users/@me', { headers: { authorization: marker } }), results.fetch);
  assert.equal(peek(), capability + marker);
  assert.equal(peek(), capability + marker);
  context.fetch('/api/v10/users/@me', { headers: { authorization: duplicate } });
  assert.equal(peek(), capability + marker);
  assert.equal(messages.length, 0);
}
for (const tokenLength of [16, 2048]) {
  const { context, peek } = setup({ linux: true });
  assert.equal(context.ipc.postMessage(capability + 'x'.repeat(tokenLength)), true);
  assert.equal(peek().length, capability.length + tokenLength);
}
for (const invalidate of ['expiry', 'origin', 'frame', 'pagehide']) {
  const { context, messages, peek, advance, pagehide } = setup({ linux: true });
  context.ipc.postMessage(capability + marker);
  if (invalidate === 'expiry') advance(600001);
  if (invalidate === 'origin') context.location.origin = 'https://evil.test';
  if (invalidate === 'frame') context.top = {};
  if (invalidate === 'pagehide') pagehide();
  assert.equal(peek(), null);
  context.location.origin = 'https://discord.com'; context.top = context; advance(0);
  assert.equal(peek(), null);
  assert.equal(context.ipc.postMessage(capability + duplicate), false);
  assert.deepEqual(messages, [true]);
}
{
  const { context, messages, advance } = setup();
  advance(600001);
  context.fetch('/api/v10/users/@me', { headers: { authorization: marker } });
  assert.equal(messages.length, 0);
}
console.log('Authentication handoff: request preservation, observer/bridge failures, repeatable bounded Linux slot, optional wakes, origin/frame/expiry/navigation checks passed (synthetic only).');
