/* Calendar Trail — service worker: cache-first app shell (data is embedded in index.html) */
const V = "calendar-trail-v3";
const SHELL = ["./", "index.html", "events.json", "manifest.webmanifest", "icon.svg", "icon-192.png", "icon-512.png"];

self.addEventListener("install", e => {
  e.waitUntil(caches.open(V).then(c => c.addAll(SHELL)).then(() => self.skipWaiting()));
});
self.addEventListener("activate", e => {
  e.waitUntil(caches.keys().then(ks => Promise.all(ks.filter(k => k !== V).map(k => caches.delete(k)))).then(() => self.clients.claim()));
});
self.addEventListener("fetch", e => {
  if (e.request.method !== "GET") return;
  const url = new URL(e.request.url);
  if (url.origin === location.origin && url.pathname.endsWith("/events.json")) {
    e.respondWith(
      fetch(e.request, { cache: "no-store" }).then(r => {
        if (!r.ok) throw new Error(`events.json: ${r.status}`);
        const copy = r.clone();
        caches.open(V).then(c => c.put(e.request, copy));
        return r;
      }).catch(() => caches.match(e.request))
    );
    return;
  }
  e.respondWith(
    caches.match(e.request).then(hit => hit || fetch(e.request).then(r => {
      if (r.ok && (url.origin === location.origin || url.origin.includes("gstatic") || url.origin.includes("googleapis"))) {
        const copy = r.clone();
        caches.open(V).then(c => c.put(e.request, copy));
      }
      return r;
    }).catch(() => url.origin === location.origin ? caches.match("index.html") : undefined))
  );
});
