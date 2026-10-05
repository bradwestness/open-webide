// BUILD_ID and SHELL_FILES are supplied by the Trunk post-build hook.
const CACHE_NAME = `openwebide-shell-${BUILD_ID}`;
const shell = new Set(SHELL_FILES);
self.addEventListener('install', event => {
  event.waitUntil(caches.open(CACHE_NAME).then(cache => cache.addAll(SHELL_FILES)));
});
self.addEventListener('activate', event => {
  event.waitUntil(caches.keys().then(keys => Promise.all(keys.filter(key => key.startsWith('openwebide-shell-') && key !== CACHE_NAME).map(key => caches.delete(key)))).then(() => self.clients.claim()));
});
self.addEventListener('fetch', event => {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== 'GET' || url.origin !== self.location.origin || url.pathname.startsWith('/api/') || url.pathname === '/api' || url.pathname.startsWith('/bridge')) return;
  if (request.mode === 'navigate') {
    event.respondWith(fetch(request, {cache: 'no-store'}).catch(async () => (await caches.open(CACHE_NAME)).match('/offline.html')));
    return;
  }
  if (!shell.has(url.pathname) || url.search) return;
  event.respondWith(caches.open(CACHE_NAME).then(async cache => (await cache.match(url.pathname)) || fetch(request)));
});
