// A span Thread's mark when the quote taken from the Markdown is not in the rendered text: a
// list item is marked at its own item (or the quoted words in it), never the whole list, and a
// nested block (a paragraph in a block quote, a table cell) at the smallest block that holds the
// anchored lines. Also the unchanged cases: an exact quote, and a repeated one.
import { test, expect } from '@playwright/test';
import { cli, harness, rangeOf, seedDoc } from './harness';
import { openDoc } from './actions';

const h = () => harness();

const LIST = `# Anchors

- First item
- Second item with **Decisions are recorded.** inside
- Third item
`;

const REPEATED = `# Repeated

- **Same** text here
- **Same** text here
`;

const NESTED = `# Nested

> A quoted paragraph with <b>raw html</b> inside.

| Col | Note |
| --- | --- |
| a | ![diagram](img/diagram.png) caption |
`;

// spec: S10-12
test('a Markdown quote inside one list item marks the quoted words, never the whole list; the card sits at the item', async ({ page }) => {
  const doc = seedDoc('web-hl-li', LIST);
  cli(h().owner, ['thread', 'new', doc.id, rangeOf(LIST, '**Decisions are recorded.**'), '-m', 'A quote with Markdown syntax.']);
  await openDoc(page, doc.id);

  // The words of the item are marked: the quote is found in the rendered text once its inline
  // Markdown is taken out.
  const li = page.locator('.cd-doc li[data-l="4-4"]');
  const mark = li.locator('mark.cd-hl[data-t="t1"]');
  await expect(mark).toHaveText('Decisions are recorded.');
  await expect(li).not.toHaveClass(/cd-hl-block/);
  // The list is not marked at all — no bar beside every bullet, no block mark.
  const ul = page.locator('.cd-doc ul[data-l="3-5"]');
  await expect(ul).not.toHaveClass(/cd-hl-block/);
  expect(await ul.getAttribute('data-t')).toBeNull();

  // The margin card aligns with the mark inside the item, not with the top of the list.
  const strip = page.locator('.cd-strip', { has: page.locator('.cd-thread[data-t="t1"]') });
  const mb = (await mark.boundingBox())!;
  const sb = (await strip.boundingBox())!;
  const lb = (await li.boundingBox())!;
  expect(sb.y).toBeGreaterThanOrEqual(mb.y - 2);
  expect(sb.y - mb.y).toBeLessThan(60);
  expect(sb.y).toBeGreaterThanOrEqual(lb.y - 2);
});

// spec: S10-12
test('a repeated Markdown quote marks the item the Thread is anchored to', async ({ page }) => {
  const doc = seedDoc('web-hl-repeat', REPEATED);
  const lines = REPEATED.split('\n');
  const second = `4:2-4:${Array.from(lines[3]).length}`;
  cli(h().owner, ['thread', 'new', doc.id, second, '-m', 'The second of two equal items.']);
  await openDoc(page, doc.id);

  const marks = page.locator('.cd-doc li[data-l="4-4"] mark.cd-hl[data-t="t1"]');
  await expect(marks.first()).toHaveText('Same');
  expect(await marks.count()).toBe(2);
  await expect(page.locator('.cd-doc li[data-l="3-3"] mark.cd-hl[data-t="t1"]')).toHaveCount(0);
  await expect(page.locator('.cd-doc ul')).not.toHaveClass(/cd-hl-block/);
});

// spec: S10-12
test('an anchor that cannot be found marks its smallest block: a paragraph in a block quote, a table cell', async ({ page }) => {
  const doc = seedDoc('web-hl-nested', NESTED);
  cli(h().owner, ['thread', 'new', doc.id, rangeOf(NESTED, '<b>raw html</b>'), '-m', 'Raw HTML in a quote.']);
  cli(h().owner, ['thread', 'new', doc.id, rangeOf(NESTED, '![diagram](img/diagram.png)'), '-m', 'An image in a table cell.']);
  await openDoc(page, doc.id);

  // The paragraph inside the block quote, not the block quote.
  const p = page.locator('.cd-doc blockquote p[data-l="3-3"]');
  await expect(p).toHaveClass(/cd-hl-block/);
  await expect(p).toHaveAttribute('data-t', 't1');
  await expect(page.locator('.cd-doc blockquote[data-l="3-3"]')).not.toHaveClass(/cd-hl-block/);
  // The cell, not the table or the row.
  const td = page.locator('.cd-doc td[data-t="t2"]');
  await expect(td).toHaveCount(1);
  await expect(td).toHaveClass(/cd-hl-block/);
  await expect(page.locator('.cd-doc table[data-l="5-7"]')).not.toHaveClass(/cd-hl-block/);
  await expect(page.locator('.cd-doc tr[data-l="7-7"]')).not.toHaveClass(/cd-hl-block/);
});

// spec: S10-12
test('an exact quote inside a list item still marks the words only', async ({ page }) => {
  const doc = seedDoc('web-hl-exact', LIST);
  cli(h().owner, ['thread', 'new', doc.id, rangeOf(LIST, 'Third item'), '-m', 'An exact quote.']);
  await openDoc(page, doc.id);

  const li = page.locator('.cd-doc li[data-l="5-5"]');
  await expect(li.locator('mark.cd-hl[data-t="t1"]')).toHaveText('Third item');
  await expect(li).not.toHaveClass(/cd-hl-block/);
  await expect(page.locator('.cd-doc ul')).not.toHaveClass(/cd-hl-block/);
});
