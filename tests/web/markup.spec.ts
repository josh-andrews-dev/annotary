// Markup in a browser: the drawing on its card above the Comment's body, in the wide layout's
// margin card and the narrow layout's open row, light and dark, at its aspect ratio and never wider
// than the card. The embed path is in `embed.spec.ts`; the markup itself in
// `crates/annotary/tests/markup_api.rs`.
import { test, expect, type Page } from '@playwright/test';
import * as path from 'node:path';
import { harness, cli, seedDoc, rangeOf, REPO_ROOT } from './harness';
import { openDoc } from './actions';

const FIX = path.join(REPO_ROOT, 'fixtures', 'markup');
const MD = '# Launch plan\n\nWe ship on Friday.\n\nThe second line goes.\n\nThe third line stays.\n';

function scene(name: string): string {
  const doc = seedDoc(name, MD);
  const mk = ['--markup', path.join(FIX, 'cross-out.png'), '--strokes', path.join(FIX, 'cross-out.strokes.json')];
  cli(harness().owner, ['thread', 'new', doc.id, rangeOf(MD, 'The second line goes.'), ...mk]);
  cli(harness().owner, ['thread', 'reply', doc.id, 't1', '-m', 'And the loop means move it.', ...mk]);
  return doc.id;
}

/** The first Markup image of t1: loaded, its box, the card's, and whether it sits above the body. */
const shown = (page: Page) => page.locator('.cd-thread[data-t="t1"] .cd-c').first().evaluate((c) => {
  const img = c.querySelector('img.cd-mk-img') as HTMLImageElement;
  const body = c.querySelector('.cd-cbody') as HTMLElement;
  const box = img.getBoundingClientRect();
  const card = c.getBoundingClientRect();
  return {
    natural: img.naturalWidth,
    w: box.width,
    h: box.height,
    cardRight: card.right,
    right: box.right,
    above: box.bottom <= body.getBoundingClientRect().top + 1,
    bg: getComputedStyle(img).backgroundColor,
    href: (img.closest('a.cd-mk') as HTMLAnchorElement).target,
  };
});

for (const scheme of ['light', 'dark'] as const) {
  // spec: S49-07
  test(`the Markup shows above the body in a margin card and a narrow row (${scheme})`, async ({ page }) => {
    const id = scene(`mk-web-${scheme}`);
    await page.emulateMedia({ colorScheme: scheme });
    await openDoc(page, id, { width: 1280 });
    const card = page.locator('.cd-thread[data-t="t1"]');
    await card.click();
    await expect(card.locator('img.cd-mk-img')).toHaveCount(2);
    await expect.poll(() => shown(page).then((s) => s.natural)).toBe(480);
    let s = await shown(page);
    expect(s.above).toBe(true);
    expect(s.right).toBeLessThanOrEqual(s.cardRight + 0.5);
    expect(Math.abs(s.w / s.h - 3)).toBeLessThan(0.05);
    expect(s.bg).toBe('rgb(255, 255, 255)');
    expect(s.href).toBe('_blank');

    await openDoc(page, id, { width: 390 });
    const row = page.locator('.cd-thread[data-t="t1"]');
    await expect(row.locator('.cd-snip')).toContainText('Markup');
    await row.click();
    await expect(row).toHaveClass(/cd-open/);
    await expect(row.locator('img.cd-mk-img').first()).toBeVisible();
    await expect.poll(() => shown(page).then((x) => x.natural)).toBe(480);
    s = await shown(page);
    expect(s.above).toBe(true);
    expect(s.right).toBeLessThanOrEqual(s.cardRight + 0.5);
    expect(Math.abs(s.w / s.h - 3)).toBeLessThan(0.05);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  });
}
