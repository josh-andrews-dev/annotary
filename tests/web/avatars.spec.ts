// Member avatars in a browser, each test on a private server (avatars and the owner's appearance
// are Tenant-wide): the animated SVG on a card, its still under reduced motion, the monogram, a
// phone's width, the dark theme; Settings → Profile (upload, refusal, clear) and the owner's Clear
// avatar on the Members screen. The embed path is in `embed.spec.ts`.
import { test, expect, type Page } from '@playwright/test';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { execFileSync } from 'node:child_process';
import { REPO_ROOT, baseEnv, binary, spawnServer, type Spawned } from './harness';

const FIX = path.join(REPO_ROOT, 'fixtures', 'avatars');
const SVG = fs.readFileSync(path.join(FIX, 'scribe.svg'));
const PNG = fs.readFileSync(path.join(FIX, 'scribe.png'));

let srv: Spawned;
test.beforeEach(async () => { srv = await spawnServer(); });
test.afterEach(async () => { await srv.stop(); });

async function call(method: string, route: string, token: string, body?: unknown): Promise<any> {
  const r = await fetch(srv.url + route, {
    method,
    headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await r.text();
  if (!r.ok) throw new Error(`${method} ${route}: ${r.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

const cliAs = (token: string, args: string[], cwd?: string) =>
  execFileSync(binary(), args, { env: { ...baseEnv(srv.home), ANNOTARY_TOKEN: token, ANNOTARY_SERVER: srv.url }, cwd, stdio: ['ignore', 'pipe', 'pipe'] }).toString();

/** A Document with `t1` by @scribe (who has the avatar) and a reply by the owner (who has none). */
async function scene(): Promise<{ doc: string; hash: string; still: string; scribe: string }> {
  const added = await call('POST', '/api/v1/members', srv.ownerToken, { handle: 'scribe', name: 'Scribe' });
  const scribe = added.token as string;
  const me = await call('PUT', '/api/v1/me/avatar', scribe, { image: SVG.toString('base64'), static: PNG.toString('base64') });
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cd-web-av-'));
  fs.writeFileSync(path.join(dir, 'av-plan.md'), '# Plan\n\nShip it soon.\n');
  const out = cliAs(srv.ownerToken, ['push', 'av-plan.md'], dir);
  const doc = /^av-plan\t(\S+)\tv1/m.exec(out)![1];
  cliAs(scribe, ['thread', 'new', doc, '--document', '-m', 'Looks right to me.']);
  cliAs(srv.ownerToken, ['thread', 'reply', doc, 't1', '-m', 'Thanks.']);
  return { doc, hash: me.avatar.hash, still: me.avatar.static_hash, scribe };
}

const imgState = (page: Page, sel: string) => page.locator(sel).first().evaluate((el) => {
  const img = el as HTMLImageElement;
  const box = img.getBoundingClientRect();
  const round = getComputedStyle(img.closest('.cd-av')!).borderRadius;
  return { src: img.currentSrc, natural: img.naturalWidth, w: box.width, h: box.height, round, tag: img.tagName };
});

// spec: S41-08
test('a card shows the animated avatar, its still under reduced motion, and a monogram for a Member without one', async ({ page }) => {
  const s = await scene();
  await page.goto(`${srv.url}/d/${s.doc}`);
  await page.waitForSelector('.cd-app.cd-ready');
  const first = page.locator('.cd-thread[data-t="t1"] .cd-c').first();
  await expect(first.locator('.cd-who .cd-av-pic img')).toBeVisible();
  await expect.poll(() => imgState(page, '.cd-thread[data-t="t1"] .cd-c .cd-av-pic img').then((i) => i.natural)).toBeGreaterThan(0);
  const shown = await imgState(page, '.cd-thread[data-t="t1"] .cd-c .cd-av-pic img');
  expect(shown.tag).toBe('IMG');
  expect(shown.src).toContain(`/avatars/${s.hash}`);
  expect([shown.w, shown.h]).toEqual([28, 28]);
  expect(shown.round).toBe('50%');

  const reply = page.locator('.cd-thread[data-t="t1"] .cd-c').nth(1);
  await expect(reply.locator('.cd-av-mono')).toHaveText('R');

  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.reload();
  await page.waitForSelector('.cd-app.cd-ready');
  await expect.poll(() => imgState(page, '.cd-thread[data-t="t1"] .cd-c .cd-av-pic img').then((i) => i.src)).toContain(`/avatars/${s.still}`);
  await expect.poll(() => imgState(page, '.cd-thread[data-t="t1"] .cd-c .cd-av-pic img').then((i) => i.natural)).toBeGreaterThan(0);
});

// spec: S41-08
test('the comment row stays on one line at a phone width, and the monogram reads on the dark theme', async ({ page }) => {
  const s = await scene();
  await call('PUT', '/api/v1/me/settings', srv.ownerToken, { key: 'appearance', value: 'dark', scope: 'all' });
  await page.setViewportSize({ width: 390, height: 800 });
  await page.goto(`${srv.url}/d/${s.doc}`);
  await page.waitForSelector('.cd-app.cd-ready');
  const card = page.locator('.cd-thread[data-t="t1"]');
  await card.click();
  await expect(card).toHaveClass(/cd-open/);
  const who = card.locator('.cd-c').first().locator('.cd-who');
  await expect(who.locator('.cd-av img')).toBeVisible();
  const box = (await who.boundingBox())!;
  expect(box.height).toBeLessThanOrEqual(30);
  const av = (await who.locator('.cd-av').boundingBox())!;
  expect(av.x + av.width).toBeLessThanOrEqual(390);

  const mono = card.locator('.cd-c').nth(1).locator('.cd-av-mono');
  const look = await mono.evaluate((el) => ({ color: getComputedStyle(el).color, bg: getComputedStyle(el).backgroundColor, page: getComputedStyle(document.querySelector('.cd-app')!).backgroundColor }));
  expect(look.color).toBe('rgb(255, 255, 255)');
  expect(look.bg).not.toBe(look.page);
});

// spec: S41-10
test('Settings → Profile uploads an avatar and its still, says why a file is refused, and clears', async ({ page }) => {
  await page.goto(`${srv.url}/settings/profile`);
  await page.waitForSelector('.cd-set-ready');
  const preview = page.locator('#cd-set-avpreview');
  await expect(preview.locator('.cd-av-mono')).toHaveText('R');
  await expect(page.locator('#cd-set-avclear')).toBeHidden();

  // Two files in either order: the SVG is the avatar, the PNG its still.
  await page.locator('#cd-set-avfile').setInputFiles([path.join(FIX, 'scribe.png'), path.join(FIX, 'scribe.svg')]);
  await expect(preview.locator('picture.cd-av-pic img')).toBeVisible();
  await expect(preview.locator('source')).toHaveCount(1);
  const me = await call('GET', '/api/v1/me', srv.ownerToken);
  expect(me.avatar.type).toBe('image/svg+xml');
  expect(me.avatar.static_type).toBe('image/png');
  await expect(page.locator('#cd-set-avclear')).toBeVisible();
  const box = (await preview.locator('.cd-av').boundingBox())!;
  expect([box.width, box.height]).toEqual([64, 64]);

  // A refused file says why and leaves the avatar alone.
  await page.locator('#cd-set-avfile').setInputFiles({ name: 'evil.svg', mimeType: 'image/svg+xml', buffer: Buffer.from('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>') });
  await expect(page.locator('#cd-set-avatar .cd-set-state.cd-set-err')).toContainText('<script>');
  await expect(preview.locator('picture.cd-av-pic img')).toBeVisible();
  expect((await call('GET', '/api/v1/me', srv.ownerToken)).avatar.hash).toBe(me.avatar.hash);

  await page.locator('#cd-set-avclear').click();
  await expect(preview.locator('.cd-av-mono')).toHaveText('R');
  await expect(page.locator('#cd-set-avclear')).toBeHidden();
  expect((await call('GET', '/api/v1/me', srv.ownerToken)).avatar).toBeNull();
});

// spec: S41-10
test('the Members screen shows avatars and an owner clears another Member’s', async ({ page }) => {
  await scene();
  await page.goto(`${srv.url}/settings/server/members`);
  const card = page.locator('.cd-mem[data-handle="scribe"]');
  await expect(card.locator('.cd-av-pic img')).toBeVisible();
  await expect(page.locator('.cd-mem[data-handle="rowan"] .cd-av-mono')).toHaveText('R');
  await expect(page.locator('.cd-mem[data-handle="rowan"] [data-act=clear-avatar]')).toHaveCount(0);
  await card.locator('[data-act=clear-avatar]').click();
  await card.locator('[data-slot=rename] [data-inline=yes]').click();
  await expect(card.locator('.cd-av-mono')).toHaveText('S');
  await expect(card.locator('[data-act=clear-avatar]')).toHaveCount(0);
  const list = await call('GET', '/api/v1/members', srv.ownerToken);
  expect(list.members.find((m: any) => m.handle === 'scribe').avatar).toBeNull();
});
