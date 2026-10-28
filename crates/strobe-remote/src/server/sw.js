"use strict";
// The remote's service worker (D517).
//
// Network first, always: while Strobe answers, every request goes to it and
// the copy here is refreshed, so an update needs no prompt and no reload.
// Only when Strobe cannot be reached does the shell come from this cache,
// so a home-screen launch opens the page and its "Disconnected" state
// rather than a browser error. The WebSocket never passes through here,
// and nothing cached can send a command: that needs the socket and a token
// Strobe still accepts.
var CACHE = "strobe-remote-1";
var SHELL = ["/", "/manifest.webmanifest", "/icon.svg", "/icon-192.png", "/icon-512.png", "/icon-maskable-512.png"];

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
  if (request.method !== "GET" || url.origin !== self.location.origin || url.pathname === "/ws") { return; }
  var key = request.mode === "navigate" ? "/" : url.pathname;
  event.respondWith(
    fetch(request)
      .then(function (response) {
        if (response.ok) {
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
