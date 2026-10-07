// BUILD_ID and SHELL_FILES are supplied by the Trunk post-build hook.
const CACHE_NAME = `openwebide-shell-${BUILD_ID}`;
const shell = new Set(SHELL_FILES);
self.addEventListener('install', event => {
  event.waitUntil(caches.open(CACHE_NAME).then(cache => cache.addAll(SHELL_FILES.map(file => new Request(new URL(file, self.location.origin), {cache: 'reload'})))));
});
self.addEventListener('activate', event => {
  event.waitUntil(caches.keys().then(keys => Promise.all(keys.filter(key => key.startsWith('openwebide-shell-') && key !== CACHE_NAME).map(key => caches.delete(key)))).then(() => self.clients.claim()));
});
self.addEventListener('fetch', event => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== 'GET' || url.origin !== self.location.origin || url.pathname.startsWith('/api/') || url.pathname === '/api' || url.pathname.startsWith('/bridge')) return;
  if (request.mode === 'navigate') {
    // Probe the server, then keep the page and its cached scripts in the same
    // build. wasm-bindgen snippet URLs can stay identical across deployments.
    event.respondWith(fetch(request, {cache: 'no-store'}).then(async response => {
      if (!response.ok) return response;
      return (await (await caches.open(CACHE_NAME)).match('/')) || response;
    }).catch(async () => (await caches.open(CACHE_NAME)).match(url.pathname === '/about.html' ? '/about.html' : '/offline.html')));
    return;
  }
  if (!shell.has(url.pathname) || url.search) return;
  event.respondWith(caches.open(CACHE_NAME).then(async cache => (await cache.match(url.pathname)) || fetch(request)));
});
