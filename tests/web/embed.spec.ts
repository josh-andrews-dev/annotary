// M3 of the Hermes plan: the list and the Document view in an embedder's sandboxed frame. A fake
// embedder stands in for the Hermes plugin: a host page (served by `page.route` at an origin of its
// own) loads `GET /embed/shell` as a `data:` URL into an `<iframe sandbox="allow-scripts">` (no
// allow-same-origin: an opaque origin, no cookies, nothing it can load by URL) and answers the
// frame's `annotary-req` messages through an exposed function that calls a real `annotary serve`
// with the owner's Bearer Token — what the plugin's backend does with `ctx.rest`. It relays no
// events unless a test posts one. Each spec spawns its own server.
import { test, expect, type Frame, type FrameLocator, type Page } from '@playwright/test';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { execFileSync } from 'node:child_process';
import { REPO_ROOT, baseEnv, binary, spawnServer, type Spawned } from './harness';
import { selectText } from './actions';

const HOST = 'http://embedder.test/host.html';
const ONE_MB = 1024 * 1024;

// The host page: mounts the frame, logs every message (type, path, size), answers requests through
// `window.annotaryBackend` (exposed by the test), and can post any message into the frame.
const HOST_HTML = `<!doctype html><html><head><meta charset="utf-8"><title>host</title></head>
<body style="margin:0"><script>
window.msgs = []; window.sizes = []; window.frameEl = null;
window.mount = function (shell, path) {
  var f = document.createElement('iframe');
  f.id = 'cd-host-frame';
  f.setAttribute('sandbox', 'allow-scripts');
  f.style.cssText = 'width:1200px;height:860px;border:0;display:block';
  f.src = 'data:text/html;charset=utf-8,' + encodeURIComponent(shell);
  f.marker = 'first-frame';
  if (path) f.addEventListener('load', function () { f.contentWindow.postMessage({ type: 'annotary-init', path: path }, '*'); }, { once: true });
  document.body.appendChild(f);
  window.frameEl = f;
};
window.toFrame = function (m) { window.frameEl.contentWindow.postMessage(m, '*'); };
window.addEventListener('message', function (e) {
  if (!window.frameEl || e.source !== window.frameEl.contentWindow) return;
  var m = e.data;
  window.sizes.push(JSON.stringify(m).length);
  window.msgs.push({ type: m.type, path: m.path || null, url: m.url || null, method: m.method || null, at: Date.now() });
  if (m.type !== 'annotary-req') return;
  window.annotaryBackend(m).then(function (r) {
    var res = { type: 'annotary-res', id: m.id, status: r.status, headers: r.headers, body: r.body };
    if (r.encoding) res.encoding = r.encoding;
    window.sizes.push(JSON.stringify(res).length);
    window.msgs.push({ type: 'annotary-res', path: m.path, status: r.status, at: Date.now() });
    e.source.postMessage(res, '*');
  });
});
</script></body></html>`;

interface Msg { type: string; path: string | null; url: string | null; method: string | null; status?: number; at: number }

let srv: Spawned;
test.beforeEach(async () => { srv = await spawnServer(); });
test.afterEach(async () => { await srv.stop(); });

function cliAs(args: string[], cwd?: string): string {
  return execFileSync(binary(), args, {
    env: { ...baseEnv(srv.home), ANNOTARY_TOKEN: srv.ownerToken, ANNOTARY_SERVER: srv.url },
    cwd,
    stdio: ['ignore', 'pipe', 'pipe'],
  }).toString();
}

function seed(name: string, md: string, files: Record<string, Buffer | string> = {}): { id: string; dir: string } {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cd-web-embed-'));
  for (const [rel, data] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
    fs.writeFileSync(path.join(dir, rel), data);
  }
  fs.writeFileSync(path.join(dir, `${name}.md`), md);
  const out = cliAs(['push', `${name}.md`], dir);
  const m = new RegExp(`^${name}\\t(\\S+)\\tv1`, 'm').exec(out);
  if (!m) throw new Error('push did not print the id: ' + out);
  return { id: m[1], dir };
}

async function api(p: string): Promise<any> {
  const r = await fetch(srv.url + p, { headers: { Authorization: `Bearer ${srv.ownerToken}` } });
  return r.json();
}

/** The fake embedder's backend: the frame's request, sent on with the Bearer Token. */
// Paths the fake backend answers as unreachable (status 0), as a plugin whose server is down would.
const unreachable = new Set<string>();

async function backend(m: { method: string; path: string; headers?: Record<string, string>; body?: string | null; binary?: boolean }) {
  if (typeof m.path !== 'string' || !m.path.startsWith('/') || m.path.startsWith('//')) return { status: 400, headers: {}, body: '' };
  if (unreachable.has(m.path)) return { status: 0, headers: {}, body: '' };
  const headers: Record<string, string> = { ...(m.headers || {}), Authorization: `Bearer ${srv.ownerToken}`, 'X-Annotary-Embed': '1' };
  try {
    const res = await fetch(srv.url + m.path, { method: m.method, headers, body: m.body ?? undefined, redirect: 'manual' });
    const hs: Record<string, string> = {};
    res.headers.forEach((v, k) => { hs[k] = v; });
    if (m.binary) return { status: res.status, headers: hs, body: Buffer.from(await res.arrayBuffer()).toString('base64'), encoding: 'base64' };
    return { status: res.status, headers: hs, body: await res.text() };
  } catch {
    return { status: 0, headers: {}, body: '' };
  }
}

/** Open the host page and mount the shell (asking for `start` with `annotary-init`). */
async function embed(page: Page, start: string | null = '/'): Promise<{ fl: FrameLocator; frame: () => Frame; msgs: () => Promise<Msg[]>; requests: string[] }> {
  const requests: string[] = [];
  page.on('request', (r) => requests.push(r.url()));
  await page.route(HOST, (r) => r.fulfill({ contentType: 'text/html', body: HOST_HTML }));
  await page.exposeFunction('annotaryBackend', backend);
  await page.setViewportSize({ width: 1240, height: 900 });
  await page.goto(HOST);
  const shell = await (await fetch(srv.url + '/embed/shell', { headers: { Authorization: `Bearer ${srv.ownerToken}` } })).text();
  await page.evaluate(([s, p]) => (window as any).mount(s, p), [shell, start] as const);
  return {
    fl: page.frameLocator('#cd-host-frame'),
    frame: () => page.frames().find((f) => f !== page.mainFrame())!,
    msgs: () => page.evaluate(() => (window as any).msgs as Msg[]),
    requests,
  };
}

const toFrame = (page: Page, m: unknown) => page.evaluate((x) => (window as any).toFrame(x), m);

const PLAN = `# Embedded plan

Ship a local server + CLI + web viewer that humans and agents can use.

We ship the CLI before the web UI so agents can start using it early.
`;

// spec: S37-06
test('the review loop runs in the frame: list → Document → comment → reply → resolve → Approve; no cookie, no request of its own', async ({ page, context }) => {
  const doc = seed('emb-flow', PLAN);
  const e = await embed(page, '/');
  const fl = e.fl;
  await expect(fl.locator('.cd-app.cd-embed.cd-listpage')).toBeVisible();
  await expect(fl.locator('#cd-nav-settings')).toHaveCount(0);
  await fl.locator('a.cd-docname', { hasText: 'emb-flow' }).click();
  await expect(fl.locator('.cd-app.cd-docpage.cd-ready')).toBeVisible();
  expect((await e.msgs()).some((m) => m.type === 'annotary-nav' && m.path === `/d/${doc.id}`)).toBe(true);

  // comment
  await selectText(e.frame(), '.cd-doc p[data-l="5-5"]', 'CLI before the web UI');
  await fl.locator('#cd-pill').click();
  const ta = fl.locator('.cd-composer textarea');
  await expect(ta).toBeFocused();
  await ta.fill('Why before?');
  await page.keyboard.press('Control+Enter');
  const t1 = fl.locator('.cd-thread[data-t="t1"]');
  await expect(t1).toBeVisible();
  await expect(t1).not.toHaveClass(/cd-pending/);
  await expect.poll(async () => JSON.stringify((await api(`/api/v1/docs/${doc.id}/threads`)).threads?.[0]?.anchor ?? null))
    .toContain('CLI before the web UI');

  // reply
  await t1.click();
  await t1.locator('[data-act="reply"]').click();
  await t1.locator('.cd-editor-reply textarea').fill('So agents can start early.');
  await page.keyboard.press('Enter');
  await expect(t1.locator('.cd-c')).toHaveCount(2);
  await expect(t1.locator('.cd-c.cd-pending')).toHaveCount(0);

  // resolve
  await t1.locator('[data-act="resolve"]').click();
  await expect.poll(async () => (await api(`/api/v1/docs/${doc.id}/threads/t1`)).resolved?.by ?? null).toBe('rowan');

  // Approve
  await fl.locator('#cd-actbtns .cd-actbtn', { hasText: 'Approve' }).click();
  await expect(fl.locator('#cd-actions .cd-chip.cd-action')).toContainText('Approve');
  await expect.poll(async () => (await api(`/api/v1/docs/${doc.id}/status`)).action?.id).toBe('approve');

  // The frame had no cookie and asked the server for nothing itself: every request went through the host.
  expect(await context.cookies()).toEqual([]);
  expect(e.requests.filter((u) => u.startsWith(srv.url))).toEqual([]);
  expect((await e.msgs()).filter((m) => m.type === 'annotary-req').length).toBeGreaterThan(5);
});

// spec: S37-06
test('a change made elsewhere arrives with no events relayed (the poll), and at once on annotary-event', async ({ page }) => {
  const doc = seed('emb-live', PLAN);
  const e = await embed(page, `/d/${doc.id}`);
  const fl = e.fl;
  await expect(fl.locator('.cd-app.cd-docpage.cd-ready.cd-live')).toBeVisible({ timeout: 10_000 });
  // Nothing is relayed: the poll finds it.
  cliAs(['thread', 'new', doc.id, '--document', '-m', 'from elsewhere']);
  await expect(fl.locator('.cd-thread', { hasText: 'from elsewhere' })).toBeVisible({ timeout: 10_000 });

  // With the page hidden the poll stops; an `annotary-event` makes it look at once.
  await e.frame().evaluate(() => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  const polls = async () => (await e.msgs()).filter((m) => m.type === 'annotary-req' && (m.path || '').startsWith('/api/v1/events')).length;
  // let an in-flight poll finish, then nothing more is asked
  await page.waitForTimeout(500);
  const before = await polls();
  cliAs(['thread', 'new', doc.id, '--document', '-m', 'second change']);
  await page.waitForTimeout(5_000);
  expect(await polls()).toBe(before);
  await expect(fl.locator('.cd-thread', { hasText: 'second change' })).toHaveCount(0);
  const at = Date.now();
  await toFrame(page, { type: 'annotary-event', payload: { type: 'thread.created', doc: doc.id } });
  await expect(fl.locator('.cd-thread', { hasText: 'second change' })).toBeVisible({ timeout: 3_000 });
  expect(Date.now() - at).toBeLessThan(3_000);
});

// spec: S37-07
test('links, the filter form, the Version select and annotary-init navigate in place; foreign messages change nothing', async ({ page }) => {
  const a = seed('emb-nav-a', PLAN);
  fs.writeFileSync(path.join(a.dir, 'emb-nav-a.md'), PLAN + '\nA second Version.\n');
  cliAs(['push', 'emb-nav-a.md'], a.dir);
  cliAs(['thread', 'new', a.id, '--document', '-m', 'thread one']);
  const b = seed('emb-nav-b', '# B\n\nThe other Document.\n');
  const e = await embed(page, '/');
  const fl = e.fl;
  await expect(fl.locator('#cd-list')).toBeVisible();

  // fields are at least 16px (iOS Safari zooms into a smaller one)
  const fs16 = await fl.locator('#cd-filters input[name="name"]').evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
  expect(fs16).toBeGreaterThanOrEqual(16);

  // the filter form, by Enter
  await fl.locator('#cd-filters input[name="name"]').fill('emb-nav-b');
  await fl.locator('#cd-filters input[name="name"]').press('Enter');
  await expect(fl.locator('tr.cd-docrow')).toHaveCount(1);
  await expect(fl.locator('tr.cd-docrow')).toHaveAttribute('data-id', b.id);
  expect((await e.msgs()).some((m) => m.type === 'annotary-nav' && (m.path || '').startsWith('/?name=emb-nav-b'))).toBe(true);
  // and by its button
  await fl.locator('#cd-filters input[name="name"]').fill('emb-nav-a');
  await fl.locator('#cd-filters button[type="submit"]').click();
  await expect(fl.locator('tr.cd-docrow')).toHaveAttribute('data-id', a.id);

  // annotary-init with another path: the same frame element, the Document at the Thread
  await toFrame(page, { type: 'annotary-init', path: `/d/${a.id}#t1` });
  await expect(fl.locator('.cd-app.cd-docpage.cd-ready')).toBeVisible();
  await expect(fl.locator('.cd-thread[data-t="t1"]')).toHaveClass(/cd-selected/);
  expect(await page.evaluate(() => [document.querySelectorAll('iframe').length, (window as any).frameEl.marker])).toEqual([1, 'first-frame']);
  const ed = await fl.locator('.cd-thread[data-t="t1"] [data-act="reply"]').evaluate((b: HTMLElement) => { b.click(); return 0; });
  expect(ed).toBe(0);
  const taSize = await fl.locator('.cd-editor-reply textarea').evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
  expect(taSize).toBeGreaterThanOrEqual(16);
  await fl.locator('.cd-editor-reply [data-act="cancel"]').click().catch(() => page.keyboard.press('Escape'));

  // the Version select
  await fl.locator('#cd-vsel').selectOption('1');
  await expect(fl.locator('#cd-banner')).toBeVisible();
  expect((await e.msgs()).some((m) => m.type === 'annotary-nav' && m.path === `/d/${a.id}/v/1`)).toBe(true);
  // a link (Go to head)
  await fl.locator('#cd-gohead').click();
  await expect(fl.locator('#cd-banner')).toHaveCount(0);
  await expect(fl.locator('.cd-app.cd-docpage.cd-ready')).toBeVisible();

  // messages of another type, or from another window, change nothing
  const navs = (await e.msgs()).length;
  await toFrame(page, { type: 'hermes-guest-loaded' });
  await page.evaluate((p) => {
    const sib = document.createElement('iframe');
    document.body.appendChild(sib);
    const f = (window as any).frameEl as HTMLIFrameElement;
    // posted by a sibling window: its source is not the frame's parent
    (sib.contentWindow as any).eval(`parent.frameEl.contentWindow.postMessage(${JSON.stringify({ type: 'annotary-init', path: p })}, '*')`);
    void f;
  }, `/d/${b.id}`);
  await page.waitForTimeout(800);
  await expect(fl.locator('.cd-app.cd-docpage')).toHaveAttribute('data-doc', a.id);
  expect((await e.msgs()).slice(navs).some((m) => m.type === 'annotary-nav')).toBe(false);
});

// spec: S37-07
test('Assets: an image comes through the bridge, links and open buttons go outside; a large Document arrives in parts under 1 MB', async ({ page }) => {
  const png = fs.readFileSync(path.join(REPO_ROOT, 'fixtures', 'assets', 'a.png'));
  const mp4 = fs.readFileSync(path.join(REPO_ROOT, 'fixtures', 'assets', 'a.mp4'));
  const md = '# Media\n\n![shot](img/shot.png)\n\n![clip](media/clip.mp4)\n\nSee [the notes](notes.txt) and [the site](https://example.com/page).\n';
  const doc = seed('emb-media', md, { 'img/shot.png': png, 'media/clip.mp4': mp4, 'notes.txt': 'notes\n' });
  let big = '# Large\n\n';
  while (big.length < 1.6 * ONE_MB) big += 'A long paragraph of the large Document, with text to cut — 東京の段落 — again and again.\n\n';
  const large = seed('emb-large', big);
  const e = await embed(page, `/d/${doc.id}`);
  const fl = e.fl;
  await expect(fl.locator('.cd-app.cd-docpage.cd-ready')).toBeVisible();

  // the image is drawn from a blob the bridge brought
  const img = fl.locator('.cd-doc img[data-cd-src]');
  await expect(img).toHaveAttribute('src', /^blob:/);
  await expect.poll(() => img.evaluate((i: HTMLImageElement) => i.naturalWidth)).toBeGreaterThan(0);
  await expect(fl.locator('.cd-doc img[data-cd-src] + button.cd-embed-open')).toBeHidden();

  // an Asset link, the video's open button, an outside link: annotary-open-external
  await fl.locator('.cd-doc a', { hasText: 'the notes' }).click();
  await fl.locator('.cd-doc button.cd-embed-open[data-cd-kind="video"]').click();
  await fl.locator('.cd-doc a', { hasText: 'the site' }).click();
  const outs = async () => (await e.msgs()).filter((m) => m.type === 'annotary-open-external').map((m) => m.path || m.url);
  await expect.poll(outs).toHaveLength(3);
  expect(await outs()).toEqual([
    expect.stringMatching(/^\/a\/[0-9a-f]{64}$/),
    expect.stringMatching(/^\/a\/[0-9a-f]{64}$/),
    'https://example.com/page',
  ]);
  await expect(fl.locator('.cd-app.cd-docpage')).toHaveAttribute('data-doc', doc.id);

  // a Document larger than one message: every message stays under 1 MB, the page comes in parts
  await toFrame(page, { type: 'annotary-init', path: `/d/${large.id}` });
  await expect(fl.locator(`.cd-app.cd-docpage[data-doc="${large.id}"].cd-ready`)).toBeVisible({ timeout: 20_000 });
  const parts = (await e.msgs()).filter((m) => m.type === 'annotary-res' && m.path === `/d/${large.id}` && m.status === 206);
  expect(parts.length).toBeGreaterThanOrEqual(4);
  const sizes: number[] = await page.evaluate(() => (window as any).sizes);
  expect(Math.max(...sizes)).toBeLessThan(ONE_MB);
  await expect(fl.locator('.cd-doc p').last()).toContainText('again and again');
});

// spec: S37-10
test('a page written over another before its script ran binds once; a failed annotary-init can be sent again; init back to the page shown keeps it', async ({ page }) => {
  const a = seed('emb-race-a', PLAN);
  const b = seed('emb-race-b', PLAN);
  cliAs(['thread', 'new', b.id, '3:1-3:5', '-m', 'on a span']);
  const e = await embed(page, `/d/${a.id}`);
  const fl = e.fl;
  await expect(fl.locator('.cd-app.cd-docpage.cd-ready')).toBeVisible();

  // Hold every frame callback (the realm outlives document.write), write A then B, then let them run.
  const held = () => e.frame().evaluate(() => {
    const w = window as any;
    w.__held = [];
    w.__realRaf = w.__realRaf || w.requestAnimationFrame;
    w.requestAnimationFrame = (f: FrameRequestCallback) => { w.__held.push(f); return 0; };
  });
  const pathNow = () => e.frame().evaluate(() => (window as any).__cdEmbedPath as string);
  await held();
  await toFrame(page, { type: 'annotary-init', path: '/' });
  await expect.poll(pathNow).toBe('/');
  await toFrame(page, { type: 'annotary-init', path: `/d/${b.id}` });
  await expect.poll(pathNow).toBe(`/d/${b.id}`);
  const queued = await e.frame().evaluate(() => (window as any).__held.length);
  expect(queued).toBeGreaterThanOrEqual(2); // both pages' boot callbacks are waiting
  await e.frame().evaluate(() => {
    const w = window as any;
    w.requestAnimationFrame = w.__realRaf;
    w.__held.splice(0).forEach((f: FrameRequestCallback) => f(performance.now()));
  });
  await expect(fl.locator('.cd-app.cd-docpage.cd-ready')).toBeVisible();
  await expect(fl.locator('mark.cd-hl')).toHaveCount(1);
  await expect(fl.locator('mark.cd-hl mark.cd-hl')).toHaveCount(0);

  // A load that failed: the toast, then the same init again loads the page.
  unreachable.add(`/d/${a.id}`);
  await toFrame(page, { type: 'annotary-init', path: `/d/${a.id}` });
  await expect(fl.locator('#cd-toast')).toContainText('could not be reached');
  unreachable.delete(`/d/${a.id}`);
  await toFrame(page, { type: 'annotary-init', path: `/d/${a.id}` });
  await expect.poll(pathNow).toBe(`/d/${a.id}`);

  // Back to the page shown while another loads: the load in flight is dropped.
  unreachable.clear();
  await held();
  await toFrame(page, { type: 'annotary-init', path: `/d/${b.id}` });
  await toFrame(page, { type: 'annotary-init', path: `/d/${a.id}` });
  await page.waitForTimeout(500);
  expect(await pathNow()).toBe(`/d/${a.id}`);
  await e.frame().evaluate(() => { const w = window as any; w.requestAnimationFrame = w.__realRaf; w.__held.splice(0).forEach((f: FrameRequestCallback) => f(performance.now())); });
});

// spec: S40-07
test('Compare and both Version selects of the diff page navigate in place; the marks render in the frame', async ({ page }) => {
  const a = seed('emb-cmp', PLAN);
  fs.writeFileSync(path.join(a.dir, 'emb-cmp.md'), PLAN.replace('early', 'right away'));
  cliAs(['push', 'emb-cmp.md'], a.dir);
  fs.writeFileSync(path.join(a.dir, 'emb-cmp.md'), PLAN.replace('early', 'right away') + '\nA closing line.\n');
  cliAs(['push', 'emb-cmp.md'], a.dir);
  const e = await embed(page, `/d/${a.id}`);
  const fl = e.fl;
  await expect(fl.locator('.cd-app.cd-embed.cd-docpage.cd-ready')).toBeVisible();
  await fl.locator('#cd-compare').click();
  await expect(fl.locator('.cd-app.cd-embed.cd-diffpage.cd-ready')).toBeVisible();
  expect((await e.msgs()).some((m) => m.type === 'annotary-nav' && m.path === `/d/${a.id}/diff?from=2&to=3`)).toBe(true);
  await expect(fl.locator('#cd-dxdoc p.cd-dx-add')).toContainText('A closing line.');
  // the selects: v1 against the head, then v1 against v2
  await fl.locator('#cd-dxfrom').selectOption('1');
  await expect(fl.locator('#cd-dxdoc ins.cd-ins', { hasText: 'right away' })).toBeVisible();
  expect((await e.msgs()).some((m) => m.type === 'annotary-nav' && m.path === `/d/${a.id}/diff?from=1&to=3`)).toBe(true);
  await fl.locator('#cd-dxto').selectOption('2');
  await expect(fl.locator('#cd-dxto')).toHaveValue('2');
  await expect(fl.locator('#cd-dxdoc p.cd-dx-add')).toHaveCount(0);
  expect((await e.msgs()).some((m) => m.type === 'annotary-nav' && m.path === `/d/${a.id}/diff?from=1&to=2`)).toBe(true);
  // the stepper works in the frame; Open v2 goes back to a Document page in place
  await fl.locator('#cd-dxnext').click();
  await expect(fl.locator('#cd-dxpos')).toHaveText('Change 1 of 1');
  await fl.locator('#cd-dxopen-to').click();
  await expect(fl.locator('.cd-app.cd-docpage.cd-ready')).toBeVisible();
  expect((await e.msgs()).some((m) => m.type === 'annotary-nav' && m.path === `/d/${a.id}/v/2`)).toBe(true);
});
