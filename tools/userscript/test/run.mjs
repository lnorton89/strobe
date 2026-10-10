// Drives send-to-strobe.user.js in Node with stubbed GM_* functions, a stubbed
// page, and a real Strobe listener if STROBE_LISTENER (host:port) and
// STROBE_TOKEN are set. Run: node tools/userscript/test/run.mjs
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import http from 'node:http';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const source = readFileSync(join(here, '..', 'send-to-strobe.user.js'), 'utf8');

const SHADER = {
  info: { id: 'XsXXDn', name: 'Test', username: 'someone', description: '', tags: [] },
  renderpass: [{ type: 'image', code: 'void mainImage(out vec4 c, in vec2 p){c=vec4(1.);}', inputs: [], outputs: [] }],
};

/** Builds a sandbox. `fetchImpl` stands in for the page's fetch. */
function sandbox({ port, token, fetchImpl, answers = {} }) {
  const store = { token, port };
  const box = {
    __STROBE_USERSCRIPT_TEST__: {},
    location: { pathname: '/view/XsXXDn', href: 'https://www.shadertoy.com/view/XsXXDn' },
    document: { querySelector: () => null, title: 'Test', getElementById: () => null },
    fetch: fetchImpl,
    prompt: () => answers.prompt ?? null,
    confirm: () => answers.confirm ?? false,
    GM_getValue: (k, d) => (k in store ? store[k] : d),
    GM_setValue: (k, v) => { store[k] = v; },
    GM_deleteValue: (k) => { delete store[k]; },
    GM_registerMenuCommand: () => {},
    GM_xmlhttpRequest: (opts) => {
      const req = http.request(opts.url, { method: opts.method, headers: opts.headers }, (res) => {
        let text = '';
        res.on('data', (c) => (text += c));
        res.on('end', () => opts.onload({ status: res.statusCode, responseText: text }));
      });
      req.on('error', () => opts.onerror());
      req.end(opts.data);
    },
    console, JSON, Promise, Array, Object, String, parseInt, setTimeout, clearTimeout,
  };
  box.globalThis = box;
  box.unsafeWindow = box;
  vm.createContext(box);
  vm.runInContext(source, box);
  return { api: box.__STROBE_USERSCRIPT_TEST__.exports, store, box };
}

const okFetch = async (url, init) => {
  assert.equal(url, '/shadertoy');
  assert.equal(init.method, 'POST');
  assert.match(init.body, /^s=%7B%22shaders%22%3A%5B%22XsXXDn%22%5D%7D&nt=1&nl=1&np=1$/);
  return { ok: true, json: async () => [SHADER] };
};

// Route 1.
{
  const { api } = sandbox({ port: 1, token: 't', fetchImpl: okFetch });
  assert.equal(api.shaderId(), 'XsXXDn');
  assert.deepEqual(await api.getShaderJson(), SHADER);
  console.log('ok  route 1 reads the shader from POST /shadertoy');
}
// Route 1 fails, route 2 (gShaderToy) answers.
{
  const { api, box } = sandbox({
    port: 1, token: 't',
    fetchImpl: async () => ({ ok: false, status: 403, json: async () => ({}) }),
  });
  box.gShaderToy = { exportToJSON: () => JSON.stringify({ info: SHADER.info, renderpass: SHADER.renderpass }) };
  assert.equal((await api.getShaderJson()).renderpass.length, 1);
  console.log('ok  route 2 falls back to gShaderToy');
}
// Both fail: the message names both and the fallback.
{
  const { api } = sandbox({ port: 1, token: 't', fetchImpl: async () => ({ ok: false, status: 403 }) });
  await assert.rejects(api.getShaderJson(), /Tried the site request.*page object.*drop it on Strobe/s);
  console.log('ok  both routes failing says what was tried');
}
// Nothing listening: a clear error.
{
  const { api } = sandbox({ port: 1, token: 't', fetchImpl: okFetch });
  await assert.rejects(api.send(SHADER, false), /Strobe did not answer on 127\.0\.0\.1:1/);
  console.log('ok  no listener gives a readable error');
}
// A fake Strobe: header, body and the 409 / replace flow.
{
  const seen = [];
  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      const parsed = JSON.parse(body);
      seen.push({ token: req.headers['x-strobe-token'], url: req.url, parsed });
      const reply = (status, obj) => {
        res.writeHead(status, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify(obj));
      };
      if (req.headers['x-strobe-token'] !== 'good') return reply(401, { error: 'bad token' });
      if (!parsed.replace) return reply(409, { title: 'Test' });
      return reply(200, { title: 'Test', warnings: ['w'], replaced: true });
    });
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const port = server.address().port;

  let s = sandbox({ port, token: 'good', fetchImpl: okFetch, answers: { confirm: true } });
  assert.match(await s.api.send(SHADER, false), /Replaced "Test" \(1 warning/);
  assert.equal(seen[0].url, '/import');
  assert.equal(seen[0].token, 'good');
  assert.equal(seen[0].parsed.pageUrl, 'https://www.shadertoy.com/view/XsXXDn');
  assert.equal(seen.length, 2, 'the 409 is followed by one replace request');

  s = sandbox({ port, token: 'good', fetchImpl: okFetch, answers: { confirm: false } });
  assert.match(await s.api.send(SHADER, false), /Kept the existing/);

  s = sandbox({ port, token: 'wrong', fetchImpl: okFetch, answers: { prompt: 'good' } });
  assert.match(await s.api.send(SHADER, true), /Replaced/);
  assert.equal(s.store.token, 'good', 'a rejected token is replaced by the prompted one');

  s = sandbox({ port, token: 'wrong', fetchImpl: okFetch, answers: { prompt: null } });
  await assert.rejects(s.api.send(SHADER, true), /did not accept the token/);
  server.close();
  console.log('ok  token, 409 replace flow, and 401 re-prompt');
}

// Against a real Strobe, when asked to.
if (process.env.STROBE_LISTENER && process.env.STROBE_TOKEN) {
  const { api } = sandbox({
    port: Number(process.env.STROBE_LISTENER.split(':')[1]),
    token: process.env.STROBE_TOKEN,
    fetchImpl: okFetch,
    answers: { confirm: true },
  });
  console.log('real Strobe:', await api.send(SHADER, false));
}
console.log('all userscript checks passed');
