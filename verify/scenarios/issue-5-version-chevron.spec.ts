// Issue #5 on the real data: the header's Version select keeps room for its native chevron, so a
// long label is ellipsized before the arrow instead of running under it, wide and at the narrower
// max width. Header screenshots land in VERIFY_OUT for the author to look at; they hold real
// Document names, so the pull request shows the corpus screenshot instead.
import { test } from '@playwright/test';
import { documents, expect, openDoc } from '../lib';

test('the Version select leaves room for its chevron on real Documents', async ({ page }) => {
  const docs = documents();
  expect(docs.length).toBeGreaterThan(0);
  const d = docs[0];
  test.info().annotations.push({ type: 'data', description: `Document ${d.id}` });
  for (const width of [1280, 1000]) {
    await openDoc(page, d.id, width);
    const sel = page.locator('#cd-vsel');
    await expect(sel).toBeVisible();
    const cs = await sel.evaluate((e) => {
      const s = getComputedStyle(e);
      return { padRight: parseFloat(s.paddingRight), overflow: s.textOverflow };
    });
    expect(cs.padRight, `width ${width}`).toBeGreaterThanOrEqual(20);
    expect(cs.overflow, `width ${width}`).toBe('ellipsis');
    const hdr = (await page.locator('.cd-vsw').boundingBox())!;
    await page.screenshot({ path: `${process.env.VERIFY_OUT}/issue-5-version-select-${width}.png`, clip: { x: 0, y: 0, width, height: hdr.y + hdr.height + 12 } });
  }
});
