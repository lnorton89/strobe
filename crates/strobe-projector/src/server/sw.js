"use strict";
// The projector page's service worker (D517).
//
// Network first, always, and never for /api: power and status go to the
// projectors every time, and the copy here is only what opens when this
// service cannot be reached. The page is kept only from a 200, so the
// login form is never what a launch shows from the cache, and the page
// holds no password (the session cookie is HttpOnly).
var CACHE = "strobe-projectors-1";
var SHELL = ["/manifest.webmanifest", "/icon.svg", "/icon-192.png", "/icon-512.png", "/icon-maskable-512.png"];

self.addEventListener("install", function (event) {
  event.waitUntil(
    caches.open(CACHE)
      .then(function (cache) { return cache.addAll(SHELL); })
      .then(function () { return self.skipWaiting(); })
  );
});

self.addEventListener("activate", function (event) {
  event.waitUntil(
    caches.keys()
      .then(function (keys) {
        return Promise.all(keys.filter(function (key) { return key !== CACHE; }).map(function (key) { return caches.delete(key); }));
      })
      .then(function () { return self.clients.claim(); })
  );
});

self.addEventListener("fetch", function (event) {
  var request = event.request;
  var url = new URL(request.url);
  if (request.method !== "GET" || url.origin !== self.location.origin || url.pathname.indexOf("/api/") === 0) { return; }
  var key = request.mode === "navigate" ? "/" : url.pathname;
  event.respondWith(
    fetch(request)
      .then(function (response) {
        if (response.status === 200 && !response.redirected) {
          var copy = response.clone();
          caches.open(CACHE).then(function (cache) { return cache.put(key, copy); });
        }
        return response;
      })
      .catch(function () {
        return caches.match(key).then(function (hit) { return hit || Response.error(); });
      })
  );
});
