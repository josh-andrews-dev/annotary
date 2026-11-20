// Markup on the real data: a Markup (the fixture drawing and its strokes) is uploaded through the
// API onto a real Document as a Thread whose root is only the drawing, and a Reply with words; the
// image is served locked down; the card shows it above the body in a browser (wide, a phone's
// width, dark); `thread show --markup DIR` writes the same PNG; deleting the root drops it.
import { test } from '@playwright/test';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { URL, OUT, cli, cliJson, documents, expect, openDoc, shot } from '../lib';

const FIX = path.resolve(__dirname, '..', '..', 'fixtures', 'markup');
const PNG = fs.readFileSync(path.join(FIX, 'cross-out.png'));
const STROKES = JSON.parse(fs.readFileSync(path.join(FIX, 'cross-out.strokes.json'), 'utf8'));

test('a Markup posted through the API shows on its Thread card and reaches the CLI', async ({ page }) => {
  const owner = cliJson<{ token: string }>(['token', 'mint', '--name', 'verify-markup']).token;
  const call = async (method: string, route: string, body?: unknown) => {
    const r = await fetch(URL + route, {
      method,
      headers: { Authorization: 'Bearer ' + owner, 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const buf = Buffer.from(await r.arrayBuffer());
    let json: any = null;
    try { json = JSON.parse(buf.toString('utf8')); } catch { /* binary */ }
    return { status: r.status, headers: r.headers, json, buf };
  };

  const doc = documents()[0];
  expect(doc).toBeTruthy();
  const up = await call('POST', `/api/v1/docs/${doc.id}/markups`, { image: PNG.toString('base64'), strokes: STROKES });
  expect(up.status).toBe(201);
  expect([up.json.width, up.json.height]).toEqual([480, 160]);
  // A damaged PNG is refused on the real server too.
  const bad = await call('POST', `/api/v1/docs/${doc.id}/markups`, { image: PNG.subarray(0, 200).toString('base64'), strokes: STROKES });
  expect([bad.status, bad.json.error]).toEqual([400, 'invalid_markup']);

  const t = await call('POST', `/api/v1/docs/${doc.id}/threads`, { subject: 'document', markup: up.json.id });
  expect(t.status).toBe(201);
  expect(t.json.markup).toEqual({ image: up.json.image, width: 480, height: 160, strokes: up.json.strokes });
  const tid = t.json.id as string;
  const r = await call('POST', `/api/v1/docs/${doc.id}/threads/${tid}/replies`, { body: 'The loop means: move this paragraph up.', markup: up.json.id });
  expect(r.status).toBe(201);

  const served = await call('GET', `/api/v1/docs/${doc.id}/markups/${up.json.image}`);
  expect(served.status).toBe(200);
  expect(served.buf.equals(PNG)).toBe(true);
  expect(served.headers.get('content-type')).toBe('image/png');
  expect(served.headers.get('x-content-type-options')).toBe('nosniff');
  expect(served.headers.get('cache-control')).toContain('immutable');

  await openDoc(page, doc.id);
  const card = page.locator(`article.cd-thread[data-t="${tid}"]`);
  await card.scrollIntoViewIfNeeded();
  await card.click();
  const img = card.locator('img.cd-mk-img').first();
  await expect(img).toBeVisible();
  await expect.poll(() => img.evaluate((el) => (el as HTMLImageElement).naturalWidth)).toBe(480);
  const above = await card.locator('.cd-c').nth(1).evaluate((c) =>
    c.querySelector('img.cd-mk-img')!.getBoundingClientRect().bottom <= c.querySelector('.cd-cbody')!.getBoundingClientRect().top + 1);
  expect(above).toBe(true);
  await page.waitForTimeout(400);
  await card.screenshot({ path: `${OUT}/markup-card.png` });
  await shot(page, 'markup-document');

  await page.emulateMedia({ colorScheme: 'dark' });
  await openDoc(page, doc.id, 390);
  const row = page.locator(`article.cd-thread[data-t="${tid}"]`);
  await row.scrollIntoViewIfNeeded();
  await expect(row.locator('.cd-snip')).toContainText('Markup');
  await row.click();
  await expect(row).toHaveClass(/cd-open/);
  await expect(row.locator('img.cd-mk-img').first()).toBeVisible();
  await page.waitForTimeout(400);
  await shot(page, 'markup-phone-dark');
  await page.emulateMedia({ colorScheme: 'light' });

  const dir = path.join(OUT, 'markup-cli');
  const text = cli(['thread', 'show', doc.id, tid, '--markup', dir]);
  expect(text).toContain(`[markup] 480×160 ${dir}/${tid}.png`);
  expect(fs.readFileSync(path.join(dir, `${tid}.png`)).equals(PNG)).toBe(true);
  expect(fs.readFileSync(path.join(dir, `${tid}.1.png`)).equals(PNG)).toBe(true);
  const list = cli(['thread', 'list', doc.id]);
  expect(list).toMatch(new RegExp(`^${tid}\\t.*\\[markup\\]`, 'm'));

  const del = await call('DELETE', `/api/v1/docs/${doc.id}/threads/${tid}`);
  expect([del.status, del.json.markup]).toEqual([200, undefined]);
});
