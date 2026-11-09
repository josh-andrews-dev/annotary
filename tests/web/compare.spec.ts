// Compare Versions: the Document page's Compare link, the diff page's default pairs and selects,
// the inline marks and their labels, the change stepper, folded unchanged blocks, the way back to
// the Document page, and a phone-width / dark-theme render. Each test pushes its own Document.
import { test, expect, type Page } from '@playwright/test';
import { cli, harness, rangeOf, seedDoc, pushVersion } from './harness';
import { openDoc } from './actions';

const V1 = `# Compare plan

We ship the beta on Friday with three people.

- one
- two

Old closing line.
`;
const V2 = V1.replace('Friday', 'Monday').replace('- two\n', '- two\n- three\n');
const V3 = V2.replace('three people', 'four people');
const V4 = V3.replace('Old closing line.\n', 'A new closing line, rewritten from scratch entirely.\n');

/** A Document with v1…v4 (and a Thread on the head, so the Document page has a card). */
function seedHistory(name: string): string {
  const d = seedDoc(name, V1);
  pushVersion(d, V2, 'Monday, and a third item');
  pushVersion(d, V3, 'four people');
  pushVersion(d, V4, 'new closing');
  cli(harness().owner, ['thread', 'new', d.id, rangeOf(V4, 'beta on Monday'), '-m', 'Is Monday firm?']);
  return d.id;
}

async function openDiff(page: Page, url: string, width = 1280) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto(url);
  await page.waitForSelector('.cd-app.cd-diffpage.cd-ready');
}

const diffUrl = (id: string, from: number, to: number) => `${harness().url}/d/${id}/diff?from=${from}&to=${to}`;

// spec: S40-05
test('the Document page has Compare only with more than one Version, with the default pair', async ({ page }) => {
  const one = seedDoc('cmp-one', V1);
  await openDoc(page, one.id);
  await expect(page.locator('#cd-compare')).toHaveCount(0);
  const id = seedHistory('cmp-link');
  await openDoc(page, id);
  await expect(page.locator('#cd-compare')).toHaveAttribute('href', `/d/${id}/diff?from=3&to=4`);
  await openDoc(page, id, { v: 2 });
  await expect(page.locator('#cd-compare')).toHaveAttribute('href', `/d/${id}/diff?from=2&to=4`);
});

// spec: S40-06
test('Compare opens the default pair, the selects pick another, the marks are labeled, and back shows the cards', async ({ page }) => {
  const id = seedHistory('cmp-flow');
  // On the head: the previous Version against it.
  await openDoc(page, id);
  await expect(page.locator('.cd-thread')).toHaveCount(1);
  await page.locator('#cd-compare').click();
  await page.waitForSelector('.cd-app.cd-diffpage.cd-ready');
  expect(new URL(page.url()).search).toBe('?from=3&to=4');
  await expect(page.locator('#cd-dxfrom')).toHaveValue('3');
  await expect(page.locator('#cd-dxto')).toHaveValue('4');
  // v3 → v4 rewrote the closing line: one removed block, one added, labeled in text.
  await expect(page.locator('#cd-dxdoc p.cd-dx-del')).toContainText('Old closing line.');
  await expect(page.locator('#cd-dxdoc p.cd-dx-del .cd-sr')).toHaveText('removed: ');
  await expect(page.locator('#cd-dxdoc p.cd-dx-add .cd-sr')).toHaveText('added: ');
  const struck = await page.locator('#cd-dxdoc p.cd-dx-del').evaluate((e) => getComputedStyle(e).textDecorationLine);
  expect(struck).toContain('line-through');
  await expect(page.locator('#cd-dxsum')).toContainText('+1 −1 lines');
  await expect(page.locator('.cd-thread')).toHaveCount(0);

  // Back on the Document page: its card is there and laid out in the margin.
  await page.goBack();
  await page.waitForSelector('.cd-app.cd-docpage.cd-ready');
  await expect(page.locator('.cd-thread')).toHaveCount(1);
  await expect(page.locator('mark.cd-hl')).toHaveCount(1);
  const top = await page.locator('.cd-strip').first().evaluate((e) => (e as HTMLElement).style.top);
  expect(top).toMatch(/px$/);

  // On an older Version: that Version against the head.
  await openDoc(page, id, { v: 1 });
  await page.locator('#cd-compare').click();
  await page.waitForSelector('.cd-app.cd-diffpage.cd-ready');
  expect(new URL(page.url()).search).toBe('?from=1&to=4');
  // Word level inside a changed paragraph, with the pseudo labels screen readers read.
  const p = page.locator('#cd-dxdoc p.cd-dx-mod').first();
  await expect(p.locator('del.cd-del')).toHaveText(['Friday', 'three']);
  await expect(p.locator('ins.cd-ins')).toHaveText(['Monday', 'four']);
  const label = await p.locator('ins.cd-ins').first().evaluate((e) => getComputedStyle(e, '::before').content);
  expect(label).toContain('added');
  const under = await p.locator('ins.cd-ins').first().evaluate((e) => getComputedStyle(e).textDecorationLine);
  expect(under).toContain('underline');
  await expect(page.locator('#cd-dxdoc li.cd-dx-add')).toHaveText('added: three');

  // The selects pick any other pair: v2 → v4 here, then v3 against v1 (shown oldest first).
  await page.locator('#cd-dxfrom').selectOption('2');
  await page.waitForURL(/from=2&to=4$/);
  await page.waitForSelector('.cd-app.cd-diffpage.cd-ready');
  await expect(page.locator('#cd-dxdoc li.cd-dx-add')).toHaveCount(0);
  await page.locator('#cd-dxto').selectOption('3');
  await page.waitForURL(/from=2&to=3$/);
  await page.waitForSelector('.cd-app.cd-diffpage.cd-ready');
  await expect(page.locator('#cd-dxdoc ins.cd-ins')).toHaveText(['four']);
  await page.goto(diffUrl(id, 3, 1));
  await expect(page.locator('#cd-dxfrom')).toHaveValue('1');
  await expect(page.locator('#cd-dxto')).toHaveValue('3');
  // The open links go to either Version's page.
  await expect(page.locator('#cd-dxopen-from')).toHaveAttribute('href', `/d/${id}/v/1`);
  await expect(page.locator('#cd-dxopen-to')).toHaveAttribute('href', `/d/${id}/v/3`);
});

// spec: S40-06
test('next and previous step through the changes; folded unchanged blocks open', async ({ page }) => {
  const body = Array.from({ length: 30 }, (_, i) => `Paragraph ${i + 1} is long enough to take a line of its own on the page.\n\n`).join('');
  const a = `# Long plan\n\n${body}The end.\n`;
  const b = `# Long plan\n\n${body.replace('Paragraph 3 is', 'Paragraph three is').replace('Paragraph 27 is', 'Paragraph twenty-seven is')}The end, changed.\n`;
  const d = seedDoc('cmp-steps', a);
  pushVersion(d, b);
  await openDiff(page, diffUrl(d.id, 1, 2));
  await expect(page.locator('#cd-dxpos')).toHaveText('3 changes');
  const fold = page.locator('details.cd-dx-same');
  await expect(fold).toHaveCount(1);
  await expect(fold).not.toHaveAttribute('open', '');
  await expect(fold.locator('summary')).toHaveText('19 unchanged blocks');

  await page.locator('#cd-dxnext').click();
  await expect(page.locator('#cd-dxpos')).toHaveText('Change 1 of 3');
  await expect(page.locator('[data-cd-dx="1"]')).toHaveClass(/cd-dx-cur/);
  await expect(page.locator('#cd-dxprev')).toBeDisabled();
  await page.locator('#cd-dxnext').click();
  await expect(page.locator('#cd-dxpos')).toHaveText('Change 2 of 3');
  await expect(page.locator('[data-cd-dx="2"]')).toBeInViewport();
  await expect(page.locator('[data-cd-dx="1"]')).not.toHaveClass(/cd-dx-cur/);
  // The keys: j next, k previous.
  await page.keyboard.press('j');
  await expect(page.locator('#cd-dxpos')).toHaveText('Change 3 of 3');
  await expect(page.locator('#cd-dxnext')).toBeDisabled();
  await expect(page.locator('[data-cd-dx="3"]')).toBeInViewport();
  await page.keyboard.press('k');
  await expect(page.locator('#cd-dxpos')).toHaveText('Change 2 of 3');

  // A fold opens on a click and shows what it held.
  await fold.locator('summary').click();
  await expect(fold).toHaveAttribute('open', '');
  await expect(fold.locator('p', { hasText: 'Paragraph 15 is' })).toBeVisible();
});

// spec: S40-04
test('a Document with one Version and the same Version twice say there is nothing to compare', async ({ page }) => {
  const one = seedDoc('cmp-single', V1);
  await openDiff(page, `${harness().url}/d/${one.id}/diff`);
  await expect(page.locator('#cd-dxnote')).toHaveText('Nothing to compare: cmp-single has one Version.');
  const d = seedDoc('cmp-same', V1);
  pushVersion(d, V2);
  await openDiff(page, diffUrl(d.id, 2, 2));
  await expect(page.locator('#cd-dxnote')).toContainText('same Version');
  await expect(page.locator('#cd-dxfrom')).toHaveValue('2');
});

// spec: S40-08
test('at a phone width nothing scrolls sideways, and the marks show in both themes', async ({ browser }) => {
  const id = seedHistory('cmp-phone');
  for (const scheme of ['light', 'dark'] as const) {
    const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, colorScheme: scheme, deviceScaleFactor: 2 });
    const page = await ctx.newPage();
    await page.goto(diffUrl(id, 1, 4));
    await page.waitForSelector('.cd-app.cd-diffpage.cd-ready');
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(overflow).toBeLessThanOrEqual(0);
    for (const sel of ['#cd-dxfrom', '#cd-dxto', '#cd-dxnext', '#cd-dxprev']) {
      const box = await page.locator(sel).boundingBox();
      expect(box, sel).not.toBeNull();
      expect(box!.x).toBeGreaterThanOrEqual(0);
      expect(box!.x + box!.width).toBeLessThanOrEqual(390);
    }
    // The intent tokens of each palette (app.css): green under added text, red under removed.
    const bg = (sel: string) => page.locator(sel).first().evaluate((e) => getComputedStyle(e).backgroundColor);
    expect(await bg('#cd-dxdoc ins.cd-ins')).toBe(scheme === 'light' ? 'rgb(220, 252, 231)' : 'rgb(15, 46, 26)');
    expect(await bg('#cd-dxdoc del.cd-del')).toBe(scheme === 'light' ? 'rgb(254, 226, 226)' : 'rgb(42, 18, 21)');
    await page.locator('#cd-dxnext').click();
    await expect(page.locator('[data-cd-dx="1"]')).toBeInViewport();
    await ctx.close();
  }
});

// spec: S40-08
test('a change far right in a wide table or a long code line does not widen the page', async ({ page }) => {
  const row = (last: string) => `| ${Array.from({ length: 8 }, (_, i) => `column ${i} text`).join(' | ')} | ${last} |`;
  const head = `| ${Array.from({ length: 9 }, (_, i) => `h${i}`).join(' | ')} |\n|${'---|'.repeat(9)}`;
  const code = (end: string) => '```\n' + 'let value = compute(alpha, beta, gamma, delta, epsilon, zeta, eta, theta, iota) + ' + end + ';\n```';
  const a = `# Wide\n\n${head}\n${row('old')}\n\n${code('one')}\n`;
  const b = `# Wide\n\n${head}\n${row('new')}\n\n${code('two')}\n`;
  const d = seedDoc('cmp-wide', a);
  pushVersion(d, b);
  await openDiff(page, diffUrl(d.id, 1, 2), 390);
  await expect(page.locator('#cd-dxdoc td.cd-dx-mod ins.cd-ins')).toHaveText('new');
  await expect(page.locator('#cd-dxdoc pre ins.cd-ins')).toHaveText('two');
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});

// spec: S43-01
test('the Version selects clip their own labels at a phone width', async ({ page }) => {
  const d = seedDoc('cmp-select-clip', V1);
  pushVersion(d, V2, 'a very long Version message that makes the option label far wider than a phone screen');
  await openDiff(page, diffUrl(d.id, 1, 2), 402);
  // WebKit counts a select's overflowing label as page width; Chromium does not, so this pins the clip.
  expect(await page.locator('.cd-dxpick').evaluate((e) => getComputedStyle(e).overflowX)).toBe('hidden');
  expect(await page.locator('#cd-dxto').evaluate((e) => getComputedStyle(e).textOverflow)).toBe('ellipsis');
  const box = await page.locator('#cd-dxto').boundingBox();
  expect(box!.x + box!.width).toBeLessThanOrEqual(402);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});

// spec: S40-13
test('a Version select works before the main script has run, and the change buttons wait for it', async ({ page }) => {
  const id = seedHistory('cmp-early');
  // The slowest device there is: `app.js` waits for a frame that never comes.
  await page.addInitScript(() => { window.requestAnimationFrame = () => 0; });
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(diffUrl(id, 1, 4));
  await expect(page.locator('#cd-dxdoc')).toBeVisible();
  await expect(page.locator('.cd-app')).not.toHaveClass(/cd-ready/);
  await page.locator('#cd-dxfrom').selectOption('2');
  await page.waitForURL(/from=2&to=4$/);
  await page.locator('#cd-dxto').selectOption('3');
  await page.waitForURL(/from=2&to=3$/);
  await expect(page.locator('#cd-dxfrom')).toHaveValue('2');
  await expect(page.locator('#cd-dxto')).toHaveValue('3');
  // Nothing has bound the change buttons yet, so they say so.
  await expect(page.locator('#cd-dxnext')).toBeDisabled();
  await expect(page.locator('#cd-dxprev')).toBeDisabled();
});

