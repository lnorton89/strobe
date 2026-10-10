// ==UserScript==
// @name         Send to Strobe
// @namespace    https://github.com/lnorton89/strobe
// @version      0.1.0
// @description  Adds a "Send to Strobe" button to Shadertoy shader pages. It sends the shader you are viewing to the Strobe VJ app running on this computer.
// @match        https://www.shadertoy.com/view/*
// @grant        GM_xmlhttpRequest
// @grant        GM_getValue
// @grant        GM_setValue
// @grant        GM_deleteValue
// @grant        GM_registerMenuCommand
// @grant        unsafeWindow
// @connect      127.0.0.1
// @run-at       document-idle
// @license      MIT
// ==/UserScript==

/*
 * Send to Strobe (docs/PLAN.md section 6.5, D115).
 *
 * What it does, in order:
 *   1. Gets the shader's JSON (see getShaderJson).
 *   2. POSTs { shader, pageUrl } to http://127.0.0.1:<port>/import with the
 *      header X-Strobe-Token, through GM_xmlhttpRequest. That call is made by
 *      the userscript manager, not the page, so the browser's cross-origin
 *      rules do not apply and Strobe sends no CORS headers at all.
 *   3. Shows what Strobe answered: imported (with any warnings), already in
 *      the library (and asks whether to replace it), or the converter's error.
 *
 * The token is made by Strobe on first run and shown in its "Import from
 * Shadertoy" panel (Browser userscript). Paste it when asked; it is kept in
 * the userscript manager's storage, and "Strobe: change token" in the
 * userscript menu replaces it.
 *
 * ------------------------------------------------------------------------
 * UNVERIFIED AGAINST THE LIVE SITE.
 * Shadertoy sits behind Cloudflare and could not be reached when this was
 * written, so how the page exposes a shader's JSON is the most likely route,
 * not a tested one (docs/OPEN_QUESTIONS.md, Q19):
 *
 *   Route 1: the same-origin request the site itself makes to load a shader,
 *            POST /shadertoy with the form body
 *              s={"shaders":["<ID>"]}&nt=1&nl=1&np=1
 *            which answers with an array of shader objects.
 *   Route 2: the page's own `gShaderToy` object, if it can describe itself.
 *
 * If both fail the button says so and names what it tried; the fallback is
 * the Shadertoy export JSON dropped onto Strobe's window. Open DevTools >
 * Network on a /view/ page and look for the request that returns the
 * shader's code to see which of these is right, then fix getShaderJson.
 * ------------------------------------------------------------------------
 */

(function () {
  'use strict';

  var DEFAULT_PORT = 47811;
  var REQUEST_TIMEOUT_MS = 130000; // Strobe may download textures first.
  var BUTTON_ID = 'strobe-send-button';

  // ---- storage -----------------------------------------------------------

  function getToken() {
    return String(GM_getValue('token', '') || '').trim();
  }

  function getPort() {
    var port = parseInt(GM_getValue('port', DEFAULT_PORT), 10);
    return port > 0 && port < 65536 ? port : DEFAULT_PORT;
  }

  function askForToken(reason) {
    var entered = prompt(
      (reason ? reason + '\n\n' : '') +
        'Paste the token from Strobe: Import from Shadertoy > Browser userscript.',
      ''
    );
    if (entered === null) return '';
    entered = entered.trim();
    if (entered) GM_setValue('token', entered);
    return entered;
  }

  // ---- getting the shader --------------------------------------------------

  function shaderId() {
    var match = /\/view\/([A-Za-z0-9]{6})(?:[/?#]|$)/.exec(location.pathname);
    return match ? match[1] : null;
  }

  /** Whether `value` looks like a Shadertoy shader (has a renderpass array). */
  function looksLikeShader(value) {
    return !!value && typeof value === 'object' && Array.isArray(value.renderpass);
  }

  /**
   * UNVERIFIED route 1. The site loads a shader with a same-origin form POST
   * to /shadertoy. The answer is an array of shader objects; the first is
   * ours. The page's own cookies and Referer are sent because this runs in
   * the page's origin.
   */
  function fromSiteRequest(id) {
    var body =
      's=' + encodeURIComponent(JSON.stringify({ shaders: [id] })) + '&nt=1&nl=1&np=1';
    return fetch('/shadertoy', {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: body,
    })
      .then(function (response) {
        if (!response.ok) throw new Error('POST /shadertoy answered HTTP ' + response.status);
        return response.json();
      })
      .then(function (json) {
        var shader = Array.isArray(json) ? json[0] : json;
        if (shader && shader.Shader) shader = shader.Shader;
        if (!looksLikeShader(shader)) {
          throw new Error('POST /shadertoy answered, but not with a shader (no renderpass)');
        }
        return shader;
      });
  }

  /**
   * UNVERIFIED route 2: the page's `gShaderToy` global, which owns the
   * running effect. Several method and property names are tried because none
   * is documented; whatever is found must still look like a shader.
   */
  function fromPageGlobal(id) {
    var page = typeof unsafeWindow !== 'undefined' ? unsafeWindow : window;
    var toy = page.gShaderToy;
    if (!toy) return Promise.reject(new Error('no gShaderToy object on this page'));
    var effect = toy.mEffect;
    var candidates = [
      function () { return toy.exportToJSON && toy.exportToJSON(); },
      function () { return effect && effect.Save && effect.Save(); },
      function () { return effect && effect.save && effect.save(); },
      function () { return toy.mShader; },
    ];
    for (var i = 0; i < candidates.length; i++) {
      try {
        var found = candidates[i]();
        if (typeof found === 'string') found = JSON.parse(found);
        if (found && found.Shader) found = found.Shader;
        if (looksLikeShader(found)) {
          // A saved effect has no `info`; fill in what the page shows.
          if (!found.info) {
            var title = document.querySelector('#shaderTitle, h1');
            found.info = {
              id: id,
              name: title ? title.textContent.trim() : document.title,
              username: '',
            };
          }
          return Promise.resolve(found);
        }
      } catch (ignored) {
        // Try the next one.
      }
    }
    return Promise.reject(new Error('gShaderToy did not give a shader'));
  }

  function getShaderJson() {
    var id = shaderId();
    if (!id) return Promise.reject(new Error('this is not a shader page'));
    return fromSiteRequest(id).catch(function (first) {
      return fromPageGlobal(id).catch(function (second) {
        throw new Error(
          'could not read the shader from this page. Tried the site request (' +
            first.message +
            ') and the page object (' +
            second.message +
            '). Export the shader JSON and drop it on Strobe instead.'
        );
      });
    });
  }

  // ---- talking to Strobe ---------------------------------------------------

  function post(shader, replace) {
    return new Promise(function (resolve, reject) {
      GM_xmlhttpRequest({
        method: 'POST',
        url: 'http://127.0.0.1:' + getPort() + '/import',
        headers: {
          'Content-Type': 'application/json',
          'X-Strobe-Token': getToken(),
        },
        data: JSON.stringify({ shader: shader, pageUrl: location.href, replace: !!replace }),
        timeout: REQUEST_TIMEOUT_MS,
        onload: function (response) {
          var body = null;
          try { body = JSON.parse(response.responseText); } catch (ignored) { /* not JSON */ }
          resolve({ status: response.status, body: body, text: response.responseText });
        },
        onerror: function () {
          reject(new Error(
            'Strobe did not answer on 127.0.0.1:' + getPort() +
              '. Is Strobe running, with "Accept shaders from the browser" switched on?'
          ));
        },
        ontimeout: function () {
          reject(new Error('Strobe took too long to answer. Check its console.'));
        },
      });
    });
  }

  /** Sends, and handles the answers that need a decision. */
  function send(shader, replace) {
    if (!getToken() && !askForToken('Strobe needs its token once.')) {
      return Promise.reject(new Error('no token: nothing was sent'));
    }
    return post(shader, replace).then(function (answer) {
      var body = answer.body || {};
      if (answer.status === 200) {
        var warnings = (body.warnings || []).length;
        return (body.replaced ? 'Replaced' : 'Imported') + ' "' + body.title + '"' +
          (warnings ? ' (' + warnings + ' warning' + (warnings === 1 ? '' : 's') +
            '; see Strobe\'s console)' : '');
      }
      if (answer.status === 409) {
        if (confirm('"' + body.title + '" is already in Strobe\'s library. Replace it?')) {
          return send(shader, true);
        }
        return 'Kept the existing "' + body.title + '"';
      }
      if (answer.status === 401) {
        GM_deleteValue('token');
        if (askForToken('Strobe did not accept the token.')) return send(shader, replace);
        throw new Error('Strobe did not accept the token');
      }
      throw new Error(body.error || ('Strobe answered HTTP ' + answer.status));
    });
  }

  // ---- the button ------------------------------------------------------------

  function toast(text, isError) {
    var el = document.getElementById('strobe-send-toast');
    if (!el) {
      el = document.createElement('div');
      el.id = 'strobe-send-toast';
      el.style.cssText =
        'position:fixed;right:16px;bottom:60px;max-width:360px;padding:8px 12px;' +
        'border-radius:6px;font:13px/1.4 sans-serif;color:#fff;z-index:2147483647;' +
        'box-shadow:0 2px 8px rgba(0,0,0,.4)';
      document.body.appendChild(el);
    }
    el.style.background = isError ? '#a33' : '#2a6';
    el.textContent = text;
    clearTimeout(el._timer);
    el._timer = setTimeout(function () { el.remove(); }, isError ? 15000 : 6000);
  }

  function click(button) {
    button.disabled = true;
    var label = button.textContent;
    button.textContent = 'Sending…';
    return getShaderJson()
      .then(function (shader) { return send(shader, false); })
      .then(function (message) { toast(message, false); })
      .catch(function (error) { toast(String(error.message || error), true); })
      .then(function () {
        button.disabled = false;
        button.textContent = label;
      });
  }

  function addButton() {
    if (document.getElementById(BUTTON_ID)) return;
    var button = document.createElement('button');
    button.id = BUTTON_ID;
    button.type = 'button';
    button.textContent = 'Send to Strobe';
    button.title = 'Import this shader into the Strobe app on this computer';
    button.style.cssText =
      'position:fixed;right:16px;bottom:16px;padding:8px 14px;border:0;border-radius:6px;' +
      'background:#e8a317;color:#111;font:600 13px sans-serif;cursor:pointer;' +
      'z-index:2147483647;box-shadow:0 2px 8px rgba(0,0,0,.4)';
    button.addEventListener('click', function () { click(button); });
    document.body.appendChild(button);
  }

  if (typeof GM_registerMenuCommand === 'function') {
    GM_registerMenuCommand('Strobe: change token', function () { askForToken(''); });
    GM_registerMenuCommand('Strobe: change port', function () {
      var entered = prompt('The port Strobe listens on (--listen-port):', String(getPort()));
      if (entered && parseInt(entered, 10) > 0) GM_setValue('port', parseInt(entered, 10));
    });
  }

  // Test hook: the Node harness in tools/userscript/test sets this global to
  // drive the script without a browser. It is inert on the real site.
  if (typeof globalThis !== 'undefined' && globalThis.__STROBE_USERSCRIPT_TEST__) {
    globalThis.__STROBE_USERSCRIPT_TEST__.exports = {
      getShaderJson: getShaderJson,
      send: send,
      looksLikeShader: looksLikeShader,
      shaderId: shaderId,
    };
    return;
  }

  addButton();
})();
