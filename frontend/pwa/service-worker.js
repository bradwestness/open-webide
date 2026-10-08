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

function validNotification(data) {
  return data && Number.isSafeInteger(data.user_id) && data.user_id > 0 && Number.isSafeInteger(data.session_id) && data.session_id > 0 &&
    typeof data.title === 'string' && data.title.length <= 256 && typeof data.body === 'string' && data.body.length <= 512 && typeof data.tag === 'string' && data.tag.startsWith(`openwebide-${data.session_id}-`) && data.tag.length <= 512;
}
async function notificationAccount(data) {
  const response = await fetch('/api/push/context', {credentials: 'same-origin', cache: 'no-store', headers: {'x-openwebide': '1'}});
  if (!response.ok) return false;
  const context = await response.json();
  return context.user_id === data.user_id && context.enabled === true;
}
async function clientContext(client) {
  return new Promise(resolve => {
    const channel = new MessageChannel();
    const timeout = setTimeout(() => {channel.port1.close(); resolve(null);}, 500);
    channel.port1.onmessage = event => {clearTimeout(timeout); channel.port1.close(); resolve(event.data);};
    client.postMessage({type: 'webide-push-context'}, [channel.port2]);
  });
}
self.addEventListener('push', event => {
  event.waitUntil((async () => {
    let data;
    try {data = event.data?.json();} catch {return;}
    if (!validNotification(data)) return;
    try {
      if (!await notificationAccount(data)) return;
      const clients = await self.clients.matchAll({type: 'window', includeUncontrolled: true});
      const contexts = await Promise.all(clients.map(clientContext));
      if (contexts.some(context => context?.user_id === data.user_id && context.session_id === data.session_id && context.chat_visible && context.focused)) return;
      await self.registration.showNotification(data.title, {body: data.body, tag: data.tag, icon: '/icon-192.png', data});
    } catch { /* No account verification means no private notification. */ }
  })());
});
self.addEventListener('notificationclick', event => {
  event.notification.close();
  event.waitUntil((async () => {
    const data = event.notification.data;
    if (!validNotification(data)) return;
    try {
      if (!await notificationAccount(data)) return;
      const clients = await self.clients.matchAll({type: 'window', includeUncontrolled: true});
      for (const client of clients) {
        const context = await clientContext(client);
        if (context?.user_id !== data.user_id) continue;
        await client.focus();
        client.postMessage({type: 'webide-open-session', user_id: data.user_id, session_id: data.session_id});
        return;
      }
      await self.clients.openWindow(`/?notification_user=${data.user_id}&notification_session=${data.session_id}`);
    } catch { /* A signed-out or different account must not open this session. */ }
  })());
});
