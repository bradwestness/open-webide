(() => {
  let pendingPrompt = null;
  const updateThemeColor = () => {
    const color = getComputedStyle(document.documentElement).getPropertyValue('--bg').trim();
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta && color) meta.content = color;
  };
  new MutationObserver(updateThemeColor).observe(document.documentElement, {attributes: true, attributeFilter: ['data-theme']});
  window.addEventListener('load', updateThemeColor);
  const installed = () => matchMedia('(display-mode: standalone)').matches || navigator.standalone === true;
  const publish = () => window.dispatchEvent(new Event('webide-install-change'));
  window.webideInstall = {
    state: () => ({secure: window.isSecureContext === true, installed: installed(), available: pendingPrompt !== null}),
    async prompt() {
      const event = pendingPrompt;
      if (!event) return false;
      pendingPrompt = null;
      publish();
      await event.prompt();
      return (await event.userChoice).outcome === 'accepted';
    }
  };
  window.addEventListener('beforeinstallprompt', event => {
    event.preventDefault();
    pendingPrompt = event;
    publish();
  });
  window.addEventListener('appinstalled', () => {pendingPrompt = null; publish();});
  matchMedia('(display-mode: standalone)').addEventListener('change', publish);
  if (window.isSecureContext && 'serviceWorker' in navigator) {
    window.addEventListener('load', () => {
      navigator.serviceWorker.register('/service-worker.js', {updateViaCache: 'none'})
        .catch(error => console.warn('App shell caching unavailable:', error));
    });
  }
})();

// Origin-bound browser primitives. Account/preferences live in the database.
(() => {
  let current = {user_id: null, enabled: false, revision: 0};
  let operations = Promise.resolve();
  let pending = null;
  const supported = () => window.isSecureContext && 'serviceWorker' in navigator && 'PushManager' in window;
  const serial = action => {const next = operations.catch(() => {}).then(action); operations = next; return next;};
  window.webidePush = {
    context(value) {
      const old = current;
      current = value;
      if (supported() && (!value.enabled || value.user_id === null) && (old.enabled || old.user_id !== null)) {
        serial(async () => {
          // A newer enable/account transition owns the subscription now.
          if (current.revision !== value.revision || current.enabled && current.user_id !== null) return;
          const registration = await navigator.serviceWorker.getRegistration();
          const subscription = await registration?.pushManager.getSubscription();
          await subscription?.unsubscribe();
          for (const notification of await registration?.getNotifications() || []) { if (notification.tag.startsWith("openwebide-")) notification.close(); }
        }).catch(() => {});
      }
    },
    subscribe(key, revision) {
      return serial(async () => {
        if (!supported() || Notification.permission !== 'granted') throw Error('Background notifications unavailable');
        let timeout;
        const registration = await Promise.race([navigator.serviceWorker.ready, new Promise((_, reject) => {timeout = setTimeout(() => reject(Error('Service worker unavailable')), 10000);})]).finally(() => clearTimeout(timeout));
        const check = () => {if (current.revision !== revision || !current.enabled || current.user_id === null) throw Error('Account changed');};
        check();
        const applicationServerKey = Uint8Array.from(atob(key.replace(/-/g, '+').replace(/_/g, '/')), char => char.charCodeAt(0));
        let subscription = await registration.pushManager.getSubscription();
        if (subscription && (new Uint8Array(subscription.options.applicationServerKey).length !== applicationServerKey.length || new Uint8Array(subscription.options.applicationServerKey).some((value, index) => value !== applicationServerKey[index]))) {
          await subscription.unsubscribe(); subscription = null;
        }
        check();
        subscription ||= await registration.pushManager.subscribe({userVisibleOnly: true, applicationServerKey});
        check();
        return subscription.toJSON();
      });
    },
    listen(callback) {
      const receive = event => {
        if (event.data?.type === 'webide-push-context') {
          event.ports[0]?.postMessage({...current, focused: document.visibilityState === 'visible' && document.hasFocus()});
        } else if (event.data?.type === 'webide-open-session') {
          callback(event.data);
        }
      };
      navigator.serviceWorker?.addEventListener('message', receive);
      if (pending) {callback(pending); pending = null;}
      return () => navigator.serviceWorker?.removeEventListener('message', receive);
    }
  };
  const url = new URL(window.location.href);
  const user_id = Number(url.searchParams.get('notification_user'));
  const session_id = Number(url.searchParams.get('notification_session'));
  if (Number.isSafeInteger(user_id) && user_id > 0 && Number.isSafeInteger(session_id) && session_id > 0) {
    pending = {user_id, session_id};
    url.searchParams.delete('notification_user'); url.searchParams.delete('notification_session');
    window.history.replaceState(null, '', url);
  }
})();
