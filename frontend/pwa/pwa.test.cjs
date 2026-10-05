const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');

function worker(fetch) {
  const handlers = {};
  const entries = new Map([['/', new Response('current build page')], ['/offline.html', new Response('Can’t reach the server')], ['/app.wasm', new Response('wasm')]]);
  const deleted = [];
  const cache = {match: async key => entries.get(key), addAll: async files => {cache.files = files;}};
  const context = {BUILD_ID: 'test-build', SHELL_FILES: ['/', '/offline.html', '/app.wasm', '/app.js'], URL, Request, fetch,
    self: {location: {origin: 'https://ide.test'}, clients: {claim: async () => {}}, addEventListener: (name, action) => {handlers[name] = action;}},
    caches: {open: async name => {assert.equal(name, 'openwebide-shell-test-build'); return cache;}, keys: async () => ['other-app', 'openwebide-shell-old', 'openwebide-shell-test-build'], delete: async key => deleted.push(key)}};
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'service-worker.js'), 'utf8'), context);
  function request(url, method = 'GET', mode = 'cors') {
    let response;
    handlers.fetch({request: {url, method, mode}, respondWith: promise => {response = promise;}});
    return response;
  }
  return {handlers, request, cache, deleted, entries};
}

test('the worker never handles API, bridge, foreign, write or unknown requests', () => {
  const host = worker(() => {throw Error('Unexpected fetch');});
  for (const route of ['/api', '/api/theme.js', '/api/sessions', '/bridge', '/bridge/exec', '/private.txt']) {
    assert.equal(host.request('https://ide.test'+route), undefined, route);
  }
  assert.equal(host.request('https://model.test/app.js'), undefined);
  assert.equal(host.request('https://ide.test/app.js', 'POST'), undefined);
  assert.equal(host.request('https://ide.test/app.js?private=1'), undefined);
});

test('versioned app assets are served from shell cache and offline navigation has guidance', async () => {
  const host = worker(async () => {throw Error('offline');});
  assert.equal(await (await host.request('https://ide.test/app.wasm')).text(), 'wasm');
  assert.match(await (await host.request('https://ide.test/', 'GET', 'navigate')).text(), /reach the server/);
  let installed;
  host.handlers.install({waitUntil: task => {installed = task;}});
  await installed;
  assert.deepEqual(Array.from(host.cache.files, request => new URL(request.url).pathname), ['/', '/offline.html', '/app.wasm', '/app.js']);
  assert.ok(host.cache.files.every(request => request.cache === 'reload'));
  let activated;
  host.handlers.activate({waitUntil: task => {activated = task;}});
  await activated;
  assert.deepEqual(host.deleted, ['openwebide-shell-old']);
});

test('navigation keeps the installed build coherent across server deployments', async () => {
  let probes = 0;
  const host = worker(async (request, options) => {
    probes++;
    assert.equal(options.cache, 'no-store');
    return new Response('new build page');
  });
  assert.equal(await (await host.request('https://ide.test/', 'GET', 'navigate')).text(), 'current build page');
  assert.equal(probes, 1);
  host.entries.delete('/');
  assert.equal(await (await host.request('https://ide.test/', 'GET', 'navigate')).text(), 'new build page');
  const failed = worker(async () => new Response('Server error', {status: 503}));
  assert.equal((await failed.request('https://ide.test/', 'GET', 'navigate')).status, 503);
});

function installation(secure, installed = false) {
  const handlers = {};
  const registrations = [];
  const window = {isSecureContext: secure, addEventListener: (name, action) => {const previous = handlers[name]; handlers[name] = event => {previous?.(event); action(event);};}, dispatchEvent: event => handlers[event.type]?.(event)};
  const navigator = {serviceWorker: {register: async (...args) => registrations.push(args)}};
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, 'pwa.js'), 'utf8'), {window, navigator, matchMedia: () => ({matches: installed, addEventListener: () => {}}), Event, console, MutationObserver: class {observe() {}}, document: {documentElement: {}, querySelector: () => null}, getComputedStyle: () => ({getPropertyValue: () => '#1e1f24'})});
  return {window, handlers, registrations};
}

test('HTTP LAN does not register a worker; secure contexts and installed display are distinct capabilities', async () => {
  const lan = installation(false);
  lan.handlers.load();
  assert.equal(lan.registrations.length, 0);
  assert.equal(lan.window.webideInstall.state().secure, false);
  const secure = installation(true);
  secure.handlers.load();
  await Promise.resolve();
  assert.equal(secure.registrations[0][0], '/service-worker.js');
  assert.equal(secure.window.webideInstall.state().available, false);
  assert.equal(installation(true, true).window.webideInstall.state().installed, true);
});

test('installation waits for a user gesture and consumes the browser prompt only once', async () => {
  const host = installation(true);
  let prompted = 0;
  host.handlers.beforeinstallprompt({preventDefault() {}, prompt: async () => {prompted++;}, userChoice: Promise.resolve({outcome: 'accepted'})});
  assert.equal(prompted, 0);
  assert.equal(host.window.webideInstall.state().available, true);
  assert.equal(await host.window.webideInstall.prompt(), true);
  assert.equal(await host.window.webideInstall.prompt(), false);
  assert.equal(prompted, 1);
});

test('phone viewport tracks an overlay keyboard and removes its listeners on disposal', () => {
  const source = fs.readFileSync(path.join(__dirname, '../src/viewport.rs'), 'utf8').match(/inline_js = r#"([\s\S]*?)"#/)[1];
  const viewport = new EventTarget();
  viewport.height = 844;
  const window = new EventTarget();
  window.innerHeight = 844;
  window.visualViewport = viewport;
  const values = new Map();
  const document = {documentElement: {style: {setProperty: (key, value) => values.set(key, value), removeProperty: key => values.delete(key)}}};
  const context = {window, document};
  vm.runInNewContext(source.replace('export function', 'function') + ';globalThis.dispose = observe_visible_height();', context);
  assert.equal(values.get('--visible-height'), '844px');
  viewport.height = 480;
  viewport.dispatchEvent(new Event('resize'));
  assert.equal(window.innerHeight, 844);
  assert.equal(values.get('--visible-height'), '480px');
  context.dispose();
  viewport.height = 844;
  viewport.dispatchEvent(new Event('resize'));
  window.dispatchEvent(new Event('resize'));
  assert.equal(values.has('--visible-height'), false);
});
