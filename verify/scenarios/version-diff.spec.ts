// Compare Versions on the real data. The real Document with the most Versions (one that predates
// this run: other scenarios push into the same copy) shows Compare on its head and on an older
// Version with the default pairs, the selects pick another pair, and the stepper walks the changes;
// screenshots at desktop and phone width, light and dark, land in VERIFY_OUT for the author (they
// hold real text, so the pull request shows the corpus screenshots instead). Then every real
// Document with more than one Version is compared head−1 → head and v1 → head: the page loads with
// no script error, nothing scrolls sideways at a phone width, and with the removed marks and labels
// taken out the diff reads exactly as the newer Version's page.
import { test, type Page } from '@playwright/test';
import { cliJson, documents, expect, openDoc, shot } from '../lib';

interface V { number: number; at: string }

const versionsOf = (id: string): V[] => cliJson<V[]>(['versions', id]) ?? [];

/** The text of the newer side of the diff (no whitespace). */
const diffText = (page: Page) =>
  page.locator('#cd-dxdoc').evaluate((root) => {
    const c = root.cloneNode(true) as HTMLElement;
    c.querySelectorAll('del, .cd-dx-del, .cd-sr, .cd-dx-sum, #cd-dxnote').forEach((e) => e.remove());
    return (c.textContent || '').replace(/\s+/g, '');
  });

/** The text of a Document page's rendered Version (no cards, no whitespace). */
const pageText = (page: Page) =>
  page.locator('#cd-doc').evaluate((root) => {
    const c = root.cloneNode(true) as HTMLElement;
    c.querySelectorAll('.cd-strip').forEach((e) => e.remove());
    return (c.textContent || '').replace(/\s+/g, '');
  });

async function openDiff(page: Page, id: string, from: number, to: number, width = 1280) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto(`/d/${id}/diff?from=${from}&to=${to}`);
  await page.waitForSelector('.cd-app.cd-diffpage.cd-ready');
}

test('Compare on real Documents: the default pairs, the selects, the stepper', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  const started = Date.now() - 10 * 60 * 1000;
  const real = documents()
    .map((d) => ({ ...d, versions: versionsOf(d.id) }))
    .filter((d) => d.versions.length > 1 && d.versions.every((v) => Date.parse(v.at) < started));
  expect(real.length, 'the live data has a Document with more than one Version').toBeGreaterThan(0);
  const d = real.reduce((a, b) => (b.versions.length > a.versions.length ? b : a));
  const head = Math.max(...d.versions.map((v) => v.number));
  test.info().annotations.push({ type: 'data', description: `Document ${d.id}, v1…v${head}` });

  // The head: the previous Version against it.
  await openDoc(page, d.id);
  await expect(page.locator('#cd-compare')).toHaveAttribute('href', `/d/${d.id}/diff?from=${head - 1}&to=${head}`);
  await page.locator('#cd-compare').click();
  await page.waitForSelector('.cd-app.cd-diffpage.cd-ready');
  await expect(page.locator('#cd-dxfrom')).toHaveValue(String(head - 1));
  await expect(page.locator('#cd-dxto')).toHaveValue(String(head));
  await expect(page.locator('#cd-dxsum')).toContainText('lines');
  await shot(page, 'version-diff-head-desktop-light');

  // An older Version: that Version against the head.
  const older = Math.max(1, head - 2);
  await openDoc(page, d.id);
  await page.goto(`/d/${d.id}/v/${older}`);
  await page.waitForSelector('.cd-app.cd-docpage.cd-ready');
  await page.locator('#cd-compare').click();
  await page.waitForSelector('.cd-app.cd-diffpage.cd-ready');
  expect(new URL(page.url()).search).toBe(`?from=${older}&to=${head}`);

  // The select: v1 against the head; the stepper walks every change.
  await page.locator('#cd-dxfrom').selectOption('1');
  await page.waitForURL(new RegExp(`from=1&to=${head}$`));
  await page.waitForSelector('.cd-app.cd-diffpage.cd-ready');
  const n = Number(await page.locator('#cd-dxsum').getAttribute('data-changes'));
  expect(n, 'v1 and the head differ').toBeGreaterThan(0);
  for (let i = 1; i <= Math.min(n, 5); i++) {
    await page.locator('#cd-dxnext').click();
    await expect(page.locator('#cd-dxpos')).toHaveText(`Change ${i} of ${n}`);
    await expect(page.locator(`[data-cd-dx="${i}"]`)).toBeInViewport();
  }
  await shot(page, 'version-diff-v1-desktop-light');

  // Phone width and the dark theme.
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await openDiff(page, d.id, 1, head, 390);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow, `${scheme}: nothing scrolls sideways`).toBeLessThanOrEqual(0);
    await page.locator('#cd-dxnext').click();
    await shot(page, `version-diff-phone-${scheme}`);
    await openDiff(page, d.id, 1, head);
    await page.locator('#cd-dxnext').click();
    await shot(page, `version-diff-desktop-${scheme}`);
  }
  await page.emulateMedia({ colorScheme: 'light' });
  expect(errors, 'no script error').toEqual([]);
});

test('every real Document with more than one Version reads back as its newer Version', async ({ page }) => {
  test.setTimeout(600_000);
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  const multi = documents()
    .map((d) => ({ ...d, versions: versionsOf(d.id) }))
    .filter((d) => d.versions.length > 1);
  let pairs = 0;
  for (const d of multi) {
    const head = Math.max(...d.versions.map((v) => v.number));
    await openDoc(page, d.id);
    const want = await pageText(page);
    for (const from of [...new Set([head - 1, 1])]) {
      await openDiff(page, d.id, from, head);
      expect(await diffText(page), `${d.id} v${from} → v${head}`).toBe(want);
      pairs++;
    }
    await page.setViewportSize({ width: 390, height: 844 });
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow, `${d.id}: nothing scrolls sideways at a phone width`).toBeLessThanOrEqual(0);
  }
  test.info().annotations.push({ type: 'data', description: `${multi.length} Documents, ${pairs} pairs` });
  expect(pairs).toBeGreaterThan(0);
  expect(errors, 'no script error').toEqual([]);
});
