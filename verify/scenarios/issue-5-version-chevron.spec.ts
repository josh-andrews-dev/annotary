// Issue #5 on the real data: a Version label too long for the header's select is ellipsized before
// the native chevron instead of running under it, wide and at the narrower max width, measured from
// the select's pixels. Only a Document that predates this run counts, and only a label that really
// overflows at that width: other scenarios push into the same copy (agent-flow's `verify-live-agent`),
// and that proves nothing. Header screenshots land in VERIFY_OUT for the author to look at; they hold
// real Document names, so the pull request shows the corpus screenshot instead.
import { test } from '@playwright/test';
import { cliJson, documents, expect, openDoc } from '../lib';
import { MIN_INK_GAP, SELECT_NARROWS_BELOW, selectInk } from '../../tests/web/select-ink';

test('the Version select leaves room for its chevron on real Documents', async ({ page }) => {
  const started = Date.now() - 10 * 60 * 1000;
  const real = documents().filter((d) => {
    const versions = cliJson<{ number: number; at: string }[]>(['versions', d.id]) ?? [];
    const head = versions.reduce((a, b) => (b.number > a.number ? b : a));
    return Date.parse(head.at) < started;
  });
  expect(real.length, 'the live data has Documents older than this run').toBeGreaterThan(0);

  let checked = 0;
  for (const width of [1280, 1000]) {
    let overflowing = false;
    for (const d of real) {
      await openDoc(page, d.id, width);
      const sel = page.locator('#cd-vsel');
      await expect(sel).toBeVisible();
      const ink = await selectInk(sel);
      if (ink.label <= ink.room) continue;
      overflowing = true;
      checked++;
      test.info().annotations.push({ type: 'data', description: `${width} px: Document ${d.id}` });
      if (width < SELECT_NARROWS_BELOW) expect(ink.width, `width ${width}`).toBeLessThanOrEqual(170);
      else expect(ink.width, `width ${width}`).toBeLessThanOrEqual(220);
      const cs = await sel.evaluate((e) => {
        const s = getComputedStyle(e);
        return { padRight: parseFloat(s.paddingRight), overflow: s.textOverflow };
      });
      expect(cs.padRight, `width ${width}`).toBeGreaterThanOrEqual(20);
      expect(cs.overflow, `width ${width}`).toBe('ellipsis');
      expect(ink.gap, `width ${width}: blank columns between the label and the chevron`).toBeGreaterThanOrEqual(MIN_INK_GAP);
      const hdr = (await page.locator('.cd-vsw').boundingBox())!;
      await page.screenshot({ path: `${process.env.VERIFY_OUT}/issue-5-version-select-${width}.png`, clip: { x: 0, y: 0, width, height: hdr.y + hdr.height + 12 } });
      break;
    }
    if (!overflowing) test.info().annotations.push({ type: 'data', description: `${width} px: no real label overflows` });
  }
  expect(checked, 'a real Version label overflows the select at one of the widths').toBeGreaterThan(0);
});
