// Embed mode (M3 of the Hermes plan) on the real data: a fake embedder loads `GET /embed/shell` as a
// `data:` URL into a sandboxed frame (no allow-same-origin) and answers its messages with a Bearer
// Token, as the Hermes plugin's backend does. The largest Document in the copy opens in the frame
// (in 206 parts when it is over one part, which needs the page to render the same bytes each time),
// with the same Threads as the ordinary page; the ordinary page keeps its Settings link and no
// `cd-embed`.
import { test, type Page } from '@playwright/test';
import { URL, cliJson, documents, expect, openDoc, shot } from '../lib';

const HOST = 'http://embedder.test/host.html';
const PART_MAX = 384 * 1024;

const HOST_HTML = `<!doctype html><html><head><meta charset="utf-8"><title>host</title></head>
<body style="margin:0"><script>
window.frameEl = null; window.biggest = 0;
window.mount = function (shell, path) {
  var f = document.createElement('iframe');
  f.id = 'cd-host-frame';
  f.setAttribute('sandbox', 'allow-scripts');
  f.style.cssText = 'width:1200px;height:860px;border:0;display:block';
  f.src = 'data:text/html;charset=utf-8,' + encodeURIComponent(shell);
  f.addEventListener('load', function () { f.contentWindow.postMessage({ type: 'annotary-init', path: path }, '*'); }, { once: true });
  document.body.appendChild(f);
  window.frameEl = f;
};
window.addEventListener('message', function (e) {
  if (!window.frameEl || e.source !== window.frameEl.contentWindow) return;
  var m = e.data;
  if (!m || m.type !== 'annotary-req') return;
  window.annotaryBackend(m).then(function (r) {
    var res = { type: 'annotary-res', id: m.id, status: r.status, headers: r.headers, body: r.body };
    if (r.encoding) res.encoding = r.encoding;
    window.biggest = Math.max(window.biggest, JSON.stringify(res).length);
    e.source.postMessage(res, '*');
  });
});
</script></body></html>`;

let token = '';
function auth(extra: Record<string, string> = {}): Record<string, string> {
  return { ...extra, Authorization: `Bearer ${token}`, 'X-Annotary-Embed': '1' };
}

async function backend(m: { method: string; path: string; headers?: Record<string, string>; body?: string | null; binary?: boolean }) {
  if (typeof m.path !== 'string' || !m.path.startsWith('/') || m.path.startsWith('//')) return { status: 400, headers: {}, body: '' };
  try {
    const res = await fetch(URL + m.path, { method: m.method, headers: auth(m.headers || {}), body: m.body ?? undefined, redirect: 'manual' });
    const hs: Record<string, string> = {};
    res.headers.forEach((v, k) => { hs[k] = v; });
    if (m.binary) return { status: res.status, headers: hs, body: Buffer.from(await res.arrayBuffer()).toString('base64'), encoding: 'base64' };
    return { status: res.status, headers: hs, body: await res.text() };
  } catch {
    return { status: 0, headers: {}, body: '' };
  }
}

async function pageBytes(id: string): Promise<number> {
  const r = await fetch(`${URL}/d/${id}`, { headers: { Authorization: `Bearer ${token}` } });
  return (await r.text()).length;
}

async function cards(page: Page): Promise<number> {
  return page.locator('.cd-card').count();
}

test('the largest real Document opens in a sandboxed embedder (in parts), with the Threads the ordinary page has', async ({ page }) => {
  token = cliJson<{ token: string }>(['token', 'mint', '--name', 'verify-embed']).token;
  expect(token).toBeTruthy();

  // the largest Document page in the copy
  let best = { id: '', bytes: 0 };
  for (const d of documents()) {
    const b = await pageBytes(d.id);
    if (b > best.bytes) best = { id: d.id, bytes: b };
  }
  expect(best.id).toBeTruthy();
  test.info().annotations.push({ type: 'data', description: `largest Document ${best.id}: ${best.bytes} bytes` });

  // asked twice, the page renders the same bytes (the parts' etag depends on it)
  const twice = await Promise.all([1, 2].map(() => fetch(`${URL}/d/${best.id}`, { headers: auth() }).then((r) => r.headers.get('x-annotary-etag') ?? r.text())));
  expect(twice[0]).toBe(twice[1]);

  // the ordinary page: no embed, Settings link, its cards
  await openDoc(page, best.id);
  await expect(page.locator('.cd-app.cd-embed')).toHaveCount(0);
  const ordinaryCards = await cards(page);

  // the same Document in the embedder
  await page.route(HOST, (r) => r.fulfill({ contentType: 'text/html', body: HOST_HTML }));
  await page.exposeFunction('annotaryBackend', backend);
  await page.goto(HOST);
  const shell = await (await fetch(`${URL}/embed/shell`, { headers: { Authorization: `Bearer ${token}` } })).text();
  await page.evaluate(([s, p]) => (window as any).mount(s, p), [shell, `/d/${best.id}`] as const);
  const fl = page.frameLocator('#cd-host-frame');
  await expect(fl.locator('.cd-app.cd-embed.cd-docpage.cd-ready')).toBeVisible({ timeout: 30_000 });
  expect(await fl.locator('.cd-card').count()).toBe(ordinaryCards);
  const biggest = await page.evaluate(() => (window as any).biggest as number);
  expect(biggest).toBeLessThan(1024 * 1024);
  if (best.bytes > PART_MAX) test.info().annotations.push({ type: 'data', description: `arrived in parts; largest message ${biggest} bytes` });
  await shot(page, 'embed-largest-document');

  // the list in the frame
  await page.evaluate(() => (window as any).frameEl.contentWindow.postMessage({ type: 'annotary-init', path: '/' }, '*'));
  await expect(fl.locator('#cd-list')).toBeVisible();
  await shot(page, 'embed-list');
});
