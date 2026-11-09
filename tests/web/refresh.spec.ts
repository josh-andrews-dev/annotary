// The margin-note visual refresh: the tie between a selected card and its highlight, the wash on a
// Comment that arrives live, mention tags, reduced motion and the one-row header. Each test pushes
// its own Document.
import { test, expect, type Page } from '@playwright/test';
import { cli, harness, pushVersion, rangeOf, seedDoc } from './harness';
import { openDoc, waitLive } from './actions';

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

// spec: S42-01
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

/** Count every `cd-wash` that starts from now on, by Comment. */
async function watchWash(page: Page) {
  await page.evaluate(() => {
    (window as any).__wash = [];
    document.addEventListener('animationstart', (e) => {
      if (e.animationName === 'cd-wash') (window as any).__wash.push((e.target as HTMLElement).dataset.c);
    }, true);
  });
}
const washes = (page: Page) => page.evaluate(() => (window as any).__wash as string[]);

// spec: S42-02
test('a Comment from someone else washes in; your own does not', async ({ page }) => {
  const id = seedThread('fresh-wash');
  await openDoc(page, id);
  await waitLive(page);
  await watchWash(page);
  const card = page.locator('.cd-thread[data-t="t1"]');
  cli(harness().agent, ['thread', 'reply', id, 't1', '-m', 'An answer from the agent.']);
  await expect(card.locator('.cd-c')).toHaveCount(2);
  await expect.poll(() => washes(page)).toHaveLength(1);
  cli(harness().owner, ['thread', 'reply', id, 't1', '-m', 'My own reply.']);
  await expect(card.locator('.cd-c')).toHaveCount(3);
  await page.waitForTimeout(200);
  expect(await washes(page)).toHaveLength(1);
});

// spec: S42-03
test('a resolved @handle in a Comment is a tag; code and unknown handles are not', async ({ page }) => {
  const id = seedThread('fresh-mention', 'Over to @rowan, not `@rowan` and not @nobody.');
  await openDoc(page, id);
  const body = page.locator('.cd-thread[data-t="t1"] .cd-cbody');
  await expect(body.locator('.cd-mention')).toHaveCount(1);
  await expect(body.locator('.cd-mention')).toHaveText('@rowan');
  await expect(body.locator('code')).toHaveText('@rowan');
});

// spec: S42-05
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

// spec: S42-07
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

// spec: S42-08
test('a long Document name does not make the list scroll sideways on a phone', async ({ page }) => {
  const name = 'release_notes_for_the_quarterly_platform_review_' + 'x'.repeat(40);
  seedThread(name);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${harness().url}/?name=release_notes`);
  await expect(page.locator('table.cd-list tr.cd-docrow')).toHaveCount(1);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
});

// spec: S42-09
test('an avatar in a card is 28 px round and keeps its own color', async ({ page }) => {
  const id = seedThread('fresh-avatar');
  await openDoc(page, id);
  const got = await page.evaluate(() => {
    const who = document.querySelector('.cd-thread[data-t="t1"] .cd-who')!;
    const read = (el: HTMLElement) => {
      who.prepend(el);
      const s = getComputedStyle(el);
      return { w: s.width, h: s.height, radius: s.borderTopLeftRadius, bg: s.backgroundColor };
    };
    const mono = document.createElement('span');
    mono.className = 'cd-av cd-av-mono cd-av-h0';
    const pic = document.createElement('picture');
    pic.className = 'cd-av';
    return { mono: read(mono), pic: read(pic) };
  });
  expect(got.mono).toEqual({ w: '28px', h: '28px', radius: '50%', bg: 'rgb(179, 64, 42)' });
  expect(got.pic).toMatchObject({ w: '28px', h: '28px', radius: '50%' });
});

// spec: S42-10
test('the tie never queries the Document, and clears when its selection goes', async ({ page }) => {
  await page.addInitScript(() => {
    const all = Element.prototype.querySelectorAll;
    (window as any).__tiedQueries = 0;
    Element.prototype.querySelectorAll = function (this: Element, sel: string) {
      if (String(sel).includes('cd-tied')) (window as any).__tiedQueries++;
      return all.call(this, sel);
    } as typeof all;
  });
  const id = seedThread('fresh-tie-clear');
  await openDoc(page, id);
  await waitLive(page);
  const tied = page.locator('.cd-strip.cd-tied');
  const mark = page.locator('mark.cd-hl[data-t="t1"]');
  await mark.click();
  await expect(tied).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(tied).toHaveCount(0);
  await mark.click();
  await expect(tied).toHaveCount(1);
  await page.setViewportSize({ width: 600, height: 900 });
  await expect(tied).toHaveCount(0);
  await page.setViewportSize({ width: 1280, height: 900 });
  await expect(tied).toHaveCount(1);
  cli(harness().agent, ['thread', 'delete', id, 't1']);
  await expect(page.locator('.cd-thread[data-t="t1"]')).toHaveCount(0);
  await expect(tied).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__tiedQueries)).toBe(0);
});

const FILLER = 'The cache keeps every rendered tile for a while, and the renderer writes through both tiers. '.repeat(16);
const LONG = `# Shared line\n\nAlpha marker opens this paragraph. ${FILLER}Omega marker closes it.\n`;

/** Two Threads starting on one source line, so one strip: t1 at its start, t2 many wrapped lines lower. */
function seedSharedStrip(name: string) {
  const d = seedDoc(name, LONG);
  cli(harness().agent, ['thread', 'new', d.id, rangeOf(LONG, 'Alpha marker'), '-m', 'First.']);
  cli(harness().agent, ['thread', 'new', d.id, rangeOf(LONG, 'Omega marker'), '-m', 'Second.']);
  return d.id;
}

// spec: S42-11
test('a strip aligns with its first visible Thread', async ({ page }) => {
  const id = seedSharedStrip('fresh-strip-visible');
  cli(harness().agent, ['thread', 'reply', id, 't1', '--resolve', '-m', 'Done.']);
  await openDoc(page, id);
  const top = (sel: string) => page.locator(sel).first().evaluate((e) => e.getBoundingClientRect().top);
  expect(Math.abs((await top('.cd-strip')) - (await top('mark.cd-hl[data-t="t2"]')))).toBeLessThanOrEqual(2);
});

// spec: S42-11
test('the tie reaches a highlight below its card', async ({ page }) => {
  const id = seedSharedStrip('fresh-tie-up');
  await openDoc(page, id);
  await page.locator('mark.cd-hl[data-t="t2"]').click();
  const strip = page.locator('.cd-strip.cd-tied');
  await expect(strip).toHaveCount(1);
  const a = await tieVar(page, '--cd-tie-a');
  const b = await tieVar(page, '--cd-tie-b');
  expect(a).toBeGreaterThan(b);
  const before = await strip.evaluate((e) => {
    const s = getComputedStyle(e, '::before');
    return { top: parseFloat(s.top), height: parseFloat(s.height), bottom: s.borderBottomWidth, topEdge: s.borderTopWidth };
  });
  expect(before.top).toBe(b);
  expect(before.height).toBe(a - b + 2);
  expect(before.bottom).toBe('2px');
  expect(before.topEdge).toBe('0px');
});

// spec: S42-12
test('the wash plays once: reopening the card does not replay it', async ({ page }) => {
  const id = seedThread('fresh-wash-once');
  await openDoc(page, id, { width: 600 });
  await waitLive(page);
  const row = page.locator('.cd-thread[data-t="t1"] .cd-row');
  await row.click();
  await watchWash(page);
  const c = page.locator('.cd-thread[data-t="t1"] .cd-c');
  cli(harness().agent, ['thread', 'reply', id, 't1', '-m', 'Live.']);
  await expect(c).toHaveCount(2);
  await expect.poll(() => washes(page)).toHaveLength(1);
  await expect(page.locator('.cd-c.cd-fresh')).toHaveCount(0, { timeout: 3000 });
  await row.click();
  await expect(c.first()).toBeHidden();
  await row.click();
  await expect(c.first()).toBeVisible();
  await page.waitForTimeout(150);
  expect(await washes(page)).toHaveLength(1);
});

// spec: S42-12
test('under reduced motion a live Comment paints at once and leaves no class behind', async ({ browser }) => {
  const id = seedThread('fresh-wash-still');
  const ctx = await browser.newContext({ reducedMotion: 'reduce', viewport: { width: 1280, height: 900 } });
  const page = await ctx.newPage();
  await openDoc(page, id);
  await waitLive(page);
  await watchWash(page);
  const c = page.locator('.cd-thread[data-t="t1"] .cd-c');
  cli(harness().agent, ['thread', 'reply', id, 't1', '-m', 'Live.']);
  await expect(c).toHaveCount(2);
  await expect(c.nth(1)).toBeVisible();
  await expect(page.locator('.cd-c.cd-fresh')).toHaveCount(0, { timeout: 3000 });
  expect(await washes(page)).toHaveLength(0);
  await ctx.close();
});

// spec: S43-03
test('a Comment with hidden tools keeps its author line on one row', async ({ page }) => {
  const id = seedThread('fresh-who-row', 'Mine to edit.', harness().owner);
  await openDoc(page, id);
  const who = page.locator('.cd-thread[data-t="t1"] .cd-who');
  await expect(who.locator('.cd-ctools .cd-link')).toHaveCount(2);
  expect((await who.boundingBox())!.height).toBeLessThanOrEqual(30);
  // hidden tools over the row's end must not take a tap
  await page.mouse.move(0, 0);
  await expect(who.locator('.cd-ctools')).toHaveCSS('pointer-events', 'none');
  await who.hover();
  await expect(who.locator('.cd-ctools')).toHaveCSS('pointer-events', 'auto');
  const tools = await who.locator('.cd-ctools').boundingBox();
  const box = await who.boundingBox();
  expect(box!.height).toBeLessThanOrEqual(30);
  expect(tools!.x + tools!.width).toBeLessThanOrEqual(box!.x + box!.width + 1);
  await expect(who.locator('.cd-ctools')).toHaveCSS('opacity', '1');
});
