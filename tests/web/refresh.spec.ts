// The margin-note visual refresh: the tie between a selected card and its highlight, the wash on a
// Comment that arrives live, mention tags, reduced motion and the one-row header. Each test pushes
// its own Document.
import { test, expect, type Page } from '@playwright/test';
import { cli, harness, pushVersion, rangeOf, seedDoc } from './harness';
import { openDoc } from './actions';

const MD = `# Refresh plan

The two-tier cache is the proposal under review here.

A second paragraph that nobody has commented on yet.
`;

function seedThread(name: string, body = 'Is this settled?', token = harness().agent) {
  const d = seedDoc(name, MD);
  cli(token, ['thread', 'new', d.id, rangeOf(MD, 'two-tier cache'), '-m', body]);
  return d.id;
}

const tieVar = (page: Page, name: string) =>
  page.locator('.cd-strip.cd-tied').evaluate((e, n) => parseFloat((e as HTMLElement).style.getPropertyValue(n)), name);

// spec: S41-01
test('the selected Thread is tied to its highlight in the margin, and only there', async ({ page }) => {
  const id = seedThread('fresh-tie');
  await openDoc(page, id);
  await expect(page.locator('.cd-strip.cd-tied')).toHaveCount(0);
  await page.locator('mark.cd-hl[data-t="t1"]').click();
  await expect(page.locator('.cd-strip.cd-tied')).toHaveCount(1);
  expect(await tieVar(page, '--cd-tie-a')).toBeLessThanOrEqual(await tieVar(page, '--cd-tie-b'));
  const before = await page.locator('.cd-strip.cd-tied').evaluate((e) => getComputedStyle(e, '::after').backgroundColor);
  expect(before).not.toBe('rgba(0, 0, 0, 0)');
  await page.keyboard.press('Escape');
  await expect(page.locator('.cd-strip.cd-tied')).toHaveCount(0);
  await page.setViewportSize({ width: 600, height: 900 });
  await page.locator('mark.cd-hl[data-t="t1"]').click();
  await expect(page.locator('.cd-thread[data-t="t1"]')).toHaveClass(/cd-selected/);
  await expect(page.locator('.cd-strip.cd-tied')).toHaveCount(0);
});

// spec: S41-02
test('a Comment from someone else washes in; your own does not', async ({ page }) => {
  const id = seedThread('fresh-wash');
  await openDoc(page, id);
  await page.waitForSelector('.cd-app.cd-live');
  const card = page.locator('.cd-thread[data-t="t1"]');
  cli(harness().agent, ['thread', 'reply', id, 't1', '-m', 'An answer from the agent.']);
  await expect(card.locator('.cd-c')).toHaveCount(2);
  await expect(card.locator('.cd-c.cd-fresh')).toHaveCount(1);
  await expect(card.locator('.cd-c.cd-fresh .cd-cbody')).toContainText('An answer from the agent.');
  cli(harness().owner, ['thread', 'reply', id, 't1', '-m', 'My own reply.']);
  await expect(card.locator('.cd-c')).toHaveCount(3);
  await expect(card.locator('.cd-c.cd-fresh')).toHaveCount(0);
});

// spec: S41-03
test('a resolved @handle in a Comment is a tag; code and unknown handles are not', async ({ page }) => {
  const id = seedThread('fresh-mention', 'Over to @rowan, not `@rowan` and not @nobody.');
  await openDoc(page, id);
  const body = page.locator('.cd-thread[data-t="t1"] .cd-cbody');
  await expect(body.locator('.cd-mention')).toHaveCount(1);
  await expect(body.locator('.cd-mention')).toHaveText('@rowan');
  await expect(body.locator('code')).toHaveText('@rowan');
});

// spec: S41-05
test('reduced motion stills the tie, and the default draws it', async ({ browser }) => {
  const id = seedThread('fresh-motion');
  const anim = async (reducedMotion: 'reduce' | 'no-preference') => {
    const ctx = await browser.newContext({ reducedMotion, baseURL: harness().url });
    const page = await ctx.newPage();
    await openDoc(page, id);
    await page.locator('mark.cd-hl[data-t="t1"]').click();
    await expect(page.locator('.cd-strip.cd-tied')).toHaveCount(1);
    const name = await page.locator('.cd-strip.cd-tied').evaluate((e) => getComputedStyle(e, '::before').animationName);
    await ctx.close();
    return name;
  };
  expect(await anim('reduce')).toBe('none');
  expect(await anim('no-preference')).toBe('cd-tie');
});

// spec: S41-07
test('with Actions and Compare the Document header stays one row at common desktop widths', async ({ page }) => {
  const d = seedDoc('fresh-header-with-a-long-enough-name', MD);
  pushVersion(d, MD.replace('nobody', 'no one'), 'a small edit to make a second Version');
  for (const width of [1280, 1366, 1440, 1600, 1920]) {
    await openDoc(page, d.id, { width });
    await expect(page.locator('#cd-actbtns .cd-actbtn').first()).toBeVisible();
    await expect(page.locator('#cd-compare')).toBeVisible();
    const box = await page.locator('.cd-hdr').boundingBox();
    expect(box!.height, `width ${width}`).toBeLessThanOrEqual(60);
  }
});

// spec: S41-08
test('a long Document name does not make the list scroll sideways on a phone', async ({ page }) => {
  const name = 'release_notes_for_the_quarterly_platform_review_' + 'x'.repeat(40);
  seedThread(name);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${harness().url}/?name=release_notes`);
  await expect(page.locator('table.cd-list tr.cd-docrow')).toHaveCount(1);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
});
