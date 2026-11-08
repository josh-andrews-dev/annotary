// The margin-note visual refresh on the real data: the list, the real Documents with the most open
// Threads and Settings load with no script error, nothing scrolls sideways at a phone width, a
// selected Thread is tied to its highlight, and the header holds one row at 1440 px. Screenshots at
// desktop and phone width, light and dark, land in VERIFY_OUT for the author (they hold real text,
// so the pull request shows corpus screenshots instead).
import { test, type Page } from '@playwright/test';
import { cliJson, expect, openDoc, shot } from '../lib';

interface Row { id: string; name: string; open?: number }

const busiest = (): Row[] =>
  (cliJson<Row[]>(['list', '--limit', '200']) ?? [])
    .filter((d) => (d.open ?? 0) > 0)
    .sort((a, b) => (b.open ?? 0) - (a.open ?? 0))
    .slice(0, 5);

function trackErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('pageerror', (e) => errors.push(e.message));
  return errors;
}

const scrollsSideways = (page: Page) =>
  page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth + 1);

test('the list and Settings render in both appearances and at a phone width', async ({ page }) => {
  const errors = trackErrors(page);
  for (const colorScheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/');
    await expect(page.locator('table.cd-list tr.cd-docrow').first()).toBeVisible();
    await shot(page, `refresh-list-desktop-${colorScheme}`);
    await page.goto('/settings/appearance');
    await expect(page.locator('#cd-set-appearance .cd-seg')).toBeVisible();
    await shot(page, `refresh-settings-desktop-${colorScheme}`);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto('/');
    await expect(page.locator('table.cd-list tr.cd-docrow').first()).toBeVisible();
    expect(await scrollsSideways(page), 'the list scrolls sideways on a phone').toBe(false);
    await shot(page, `refresh-list-phone-${colorScheme}`);
  }
  expect(errors).toEqual([]);
});

test('real Documents: one header row, a tied selection, nothing sideways on a phone', async ({ page }) => {
  const docs = busiest();
  test.skip(docs.length === 0, 'the copy has no open Threads');
  const errors = trackErrors(page);
  for (const [i, d] of docs.entries()) {
    await openDoc(page, d.id, 1440);
    const header = await page.locator('.cd-hdr').boundingBox();
    expect(header!.height, `${d.name}: the header wraps at 1440`).toBeLessThanOrEqual(60);
    const mark = page.locator('mark.cd-hl:not(.cd-res)').first();
    if (await mark.count()) {
      await mark.click();
      await expect(page.locator('.cd-strip.cd-tied'), `${d.name}: no tie`).toHaveCount(1);
    }
    if (i === 0) {
      for (const colorScheme of ['light', 'dark'] as const) {
        await page.emulateMedia({ colorScheme });
        await page.waitForTimeout(300); // the hover transitions also fade a change of appearance
        await shot(page, `refresh-doc-desktop-${colorScheme}`);
      }
      await page.emulateMedia({ colorScheme: 'light' });
    }
    await openDoc(page, d.id, 390);
    expect(await scrollsSideways(page), `${d.name}: scrolls sideways on a phone`).toBe(false);
    if (i === 0) await shot(page, 'refresh-doc-phone-light');
  }
  expect(errors).toEqual([]);
});
