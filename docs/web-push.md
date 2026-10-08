# Web Push

Enable **Browser notifications** in Settings and allow the browser permission.
The app registers that browser with the server automatically. Local and remote projects and
projectless chats can then notify you when a run finishes or needs approval,
including approvals from child agents, even after the app window closes. The
notification names the project and session and summarizes the actual result;
clicking opens its chat. A focused, visible chat suppresses its notification.
Local runs can continue with the app closed when a paired execution host is
running the task; browser-only execution still needs the app open.

Use HTTPS (or localhost for development), keep Spin and the execution bridge
running, and stay signed in. Push support depends on the browser; iOS/iPadOS
requires installing the app on the Home Screen. Unsupported browsers or failed
subscription setup retain app-open notifications; Settings indicates whether
background delivery is enabled. Permission is granted separately on each device.
Disabling notifications saves the account preference in the database and stops
queued delivery on every device. Logout invalidates account subscriptions.

No paid developer account or separate notification hosting is needed. Spin
creates and persists its VAPID signing key in SQLite, encrypts each notification,
and sends it to the browser's push provider. The bridge wakes the shared delivery
queue every five seconds. Back up the database to retain the key and subscriptions.
The optional `web_push_contact` Spin variable defaults to
`https://openwebide.com`; operators may set it to their own HTTPS contact URL or
`mailto:` address. Endpoint tokens and encryption keys are never logged.

Delivery is best effort: finished-run alerts expire after one hour, approval
alerts after five minutes or when resolved, and temporary failures retry up to
six attempts. Invalid/expired subscriptions are removed. The service worker
checks the signed-in account and saved preference before displaying or opening
an alert; without that verification it displays nothing. Completion alerts may include a short summary or excerpt of the final response.
Hidden reasoning and tool arguments are excluded. The assistance model can
summarize the response and tool outcomes; failures retain a bounded excerpt.
