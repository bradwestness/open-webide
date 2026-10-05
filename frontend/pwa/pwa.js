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
