// Member avatars on the real data: a fresh Member sets an animated SVG and its still with their own
// Token, comments on a real Document, and the card shows the avatar in a browser (the still under
// reduced motion, at a phone's width, in dark); `/avatars/{hash}` answers with its locked-down
// headers; Settings shows it; clearing it brings the monogram back and the URL goes away.
import { test } from '@playwright/test';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { URL, OUT, cliJson, documents, expect, openDoc, shot } from '../lib';

const FIX = path.resolve(__dirname, '..', '..', 'fixtures', 'avatars');
const b64 = (name: string) => fs.readFileSync(path.join(FIX, name)).toString('base64');

test('a fresh Member gets an animated avatar on a real Document, served locked down, then clears it', async ({ page }) => {
  const owner = cliJson<{ token: string }>(['token', 'mint', '--name', 'verify-avatars']).token;
  const call = async (method: string, route: string, token: string, body?: unknown) => {
    const r = await fetch(URL + route, {
      method,
      headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    return { status: r.status, headers: r.headers, json: await r.clone().json().catch(() => null) };
  };
  const handle = `verify-av-${Date.now().toString(36)}`;
  const added = await call('POST', '/api/v1/members', owner, { handle, name: 'Verify Avatar' });
  expect(added.status).toBe(201);
  const token = added.json.token as string;

  const put = await call('PUT', '/api/v1/me/avatar', token, { image: b64('scribe.svg'), static: b64('scribe.png') });
  expect(put.status).toBe(200);
  const avatar = put.json.avatar;
  expect(avatar.type).toBe('image/svg+xml');

  // An SVG with a script is refused on the real server too.
  const evil = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>').toString('base64');
  const refused = await call('PUT', '/api/v1/me/avatar', token, { image: evil });
  expect([refused.status, refused.json.error]).toEqual([400, 'invalid_avatar']);

  const doc = documents()[0];
  expect(doc).toBeTruthy();
  const made = await call('POST', `/api/v1/docs/${doc.id}/threads`, token, { subject: 'document', body: 'Checking my avatar on a real Document.' });
  expect(made.status).toBe(201);

  const served = await call('GET', avatar.url, token);
  expect(served.status).toBe(200);
  expect(served.headers.get('content-type')).toBe('image/svg+xml');
  expect(served.headers.get('x-content-type-options')).toBe('nosniff');
  expect(served.headers.get('content-security-policy')).toBe("default-src 'none'; style-src 'unsafe-inline'; img-src data:");
  expect(served.headers.get('cache-control')).toContain('immutable');

  await openDoc(page, doc.id);
  const comment = page.locator(`.cd-c:has(.cd-member[title="@${handle}"])`).first();
  await comment.scrollIntoViewIfNeeded();
  const img = comment.locator('.cd-av-pic img');
  await expect(img).toBeVisible();
  await expect.poll(() => img.evaluate((el) => (el as HTMLImageElement).naturalWidth)).toBeGreaterThan(0);
  expect(await img.evaluate((el) => (el as HTMLImageElement).currentSrc)).toContain(avatar.url);
  await page.waitForTimeout(700);
  await page.locator(`article.cd-thread:has(.cd-member[title="@${handle}"])`).first().screenshot({ path: `${OUT}/avatars-card.png` });
  await shot(page, 'avatars-document');

  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.reload();
  await page.waitForSelector('.cd-app.cd-ready');
  await expect.poll(() => img.evaluate((el) => (el as HTMLImageElement).currentSrc)).toContain(avatar.static_url);
  await page.emulateMedia({ reducedMotion: 'no-preference' });

  await page.setViewportSize({ width: 390, height: 800 });
  await page.reload();
  await page.waitForSelector('.cd-app.cd-ready');
  const card = page.locator(`article.cd-thread:has(.cd-member[title="@${handle}"])`).first();
  await card.scrollIntoViewIfNeeded();
  await card.click();
  await expect(card).toHaveClass(/cd-open/);
  await expect(card.locator('.cd-av-pic img').first()).toBeVisible();
  await page.waitForTimeout(400);
  await shot(page, 'avatars-phone');

  const before = (await call('GET', '/api/v1/me/settings', owner)).json.settings.find((s: { key: string }) => s.key === 'appearance').value;
  expect((await call('PUT', '/api/v1/me/settings', owner, { key: 'appearance', value: 'dark', scope: 'all' })).status).toBe(200);
  await openDoc(page, doc.id);
  await comment.scrollIntoViewIfNeeded();
  await page.waitForTimeout(700);
  await page.locator(`article.cd-thread:has(.cd-member[title="@${handle}"])`).first().screenshot({ path: `${OUT}/avatars-card-dark.png` });
  expect((await call('PUT', '/api/v1/me/settings', owner, { key: 'appearance', value: before, scope: 'all' })).status).toBe(200);

  await page.goto('/settings/server/members');
  const memberCard = page.locator(`.cd-mem[data-handle="${handle}"]`);
  await expect(memberCard.locator('.cd-av-pic img')).toBeVisible();
  await memberCard.scrollIntoViewIfNeeded();
  await memberCard.screenshot({ path: `${OUT}/avatars-member-card.png` });

  await page.setViewportSize({ width: 1100, height: 700 });
  const mine = await call('PUT', '/api/v1/me/avatar', owner, { image: b64('linter.svg'), static: b64('linter.png') });
  expect(mine.status).toBe(200);
  await page.goto('/settings/profile');
  await page.waitForSelector('.cd-set-ready');
  await expect(page.locator('#cd-set-avpreview picture.cd-av-pic img')).toBeVisible();
  await page.waitForTimeout(500);
  await shot(page, 'avatars-settings');
  expect((await call('DELETE', '/api/v1/me/avatar', owner)).status).toBe(200);

  expect((await call('DELETE', '/api/v1/me/avatar', token)).json.changed).toBe(true);
  await openDoc(page, doc.id);
  await expect(comment.locator('.cd-av-mono')).toHaveText('V');
  expect((await call('GET', avatar.url, token)).status).toBe(404);
});
