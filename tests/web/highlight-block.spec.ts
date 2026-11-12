// A span Thread's mark when the quote taken from the Markdown is not in the rendered text: a
// list item is marked at its own item (or the quoted words in it), never the whole list, and a
// nested block (a paragraph in a block quote, a table cell or row) at the smallest unambiguous
// block that holds the anchored lines. Also the unchanged cases: an exact quote, a repeated one,
// and a plain form that is ambiguous on its line (which falls back to the block).
import { test, expect, type Page } from '@playwright/test';
import { cli, harness, rangeOf, seedDoc } from './harness';
import { openDoc } from './actions';

const h = () => harness();

const LIST = `# Anchors

- First item
- Second item with **Decisions are recorded.** inside
- Third item
`;

const ESCAPES = `# Escapes

- Code span with \`a\\*b\` inside.
- Literal a \\*b\\* c marks.
`;

const REPEATED = `# Repeated

- **Same** text here
- **Same** text here
`;

const NESTED = `# Nested

> A quoted paragraph with <b>raw html</b> inside.

| Col |
| --- |
| ![diagram](img/diagram.png) |

| Left | Right |
| --- | --- |
| ![diagram](img/diagram.png) | note |
`;

const LOOSE = `# Loose

- ![diagram](img/diagram.png) First paragraph with text.

  Second paragraph of the same item.

- Outer item
  - ![diagram](img/diagram.png) Inner item with text.
`;

/** `L:C-L:C` of the `nth` (0-based) occurrence of `needle` in `md` (1-based lines, 0-based code points). */
function nthRange(md: string, needle: string, nth: number): string {
  const lines = md.split('\n');
  let seen = 0;
  for (let i = 0; i < lines.length; i++) {
    for (let k = lines[i].indexOf(needle); k >= 0; k = lines[i].indexOf(needle, k + 1)) {
      if (seen++ === nth) {
        const col = Array.from(lines[i].slice(0, k)).length;
        return `${i + 1}:${col}-${i + 1}:${col + Array.from(needle).length}`;
      }
    }
  }
  throw new Error(`occurrence ${nth} of "${needle}" not in the Markdown`);
}

async function assertCardAt(page: Page, sel: string, tid: string) {
  const strip = page.locator('.cd-strip').filter({ has: page.locator(`.cd-thread[data-t="${tid}"]`) }).first();
  const box = (await page.locator(sel).boundingBox())!;
  const sb = (await strip.boundingBox())!;
  expect(sb.y).toBeGreaterThanOrEqual(box.y - 2);
  expect(sb.y - box.y).toBeLessThan(60);
}

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
  const mb = (await mark.boundingBox())!;
  const lb = (await li.boundingBox())!;
  await assertCardAt(page, '.cd-doc li[data-l="4-4"]', 't1');
  const sb = (await page.locator('.cd-strip', { has: page.locator('.cd-thread[data-t="t1"]') }).first().boundingBox())!;
  expect(sb.y).toBeGreaterThanOrEqual(mb.y - 2);
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
test('an anchor that cannot be found marks its smallest block: a paragraph in a block quote and a cell of a one-cell row', async ({ page }) => {
  const doc = seedDoc('web-hl-nested', NESTED);
  cli(h().owner, ['thread', 'new', doc.id, rangeOf(NESTED, '<b>raw html</b>'), '-m', 'Raw HTML in a quote.']);
  cli(h().owner, ['thread', 'new', doc.id, nthRange(NESTED, '![diagram](img/diagram.png)', 0), '-m', 'An image in a one-cell row.']);
  await openDoc(page, doc.id);

  // The paragraph inside the block quote, not the block quote.
  const p = page.locator('.cd-doc blockquote p[data-l="3-3"]');
  await expect(p).toHaveClass(/cd-hl-block/);
  await expect(p).toHaveAttribute('data-t', 't1');
  await expect(page.locator('.cd-doc blockquote[data-l="3-3"]')).not.toHaveClass(/cd-hl-block/);
  // One cell in the row: the cell, not the table or the row.
  const td = page.locator('.cd-doc td[data-t="t2"]');
  await expect(td).toHaveCount(1);
  await expect(td).toHaveClass(/cd-hl-block/);
  await expect(page.locator('.cd-doc table[data-l="5-7"]')).not.toHaveClass(/cd-hl-block/);
  await expect(page.locator('.cd-doc tr[data-l="7-7"]')).not.toHaveClass(/cd-hl-block/);
});

// spec: S10-12
test('a table cell that shares its row lines with another cell marks the row, not the last cell', async ({ page }) => {
  const doc = seedDoc('web-hl-table', NESTED);
  const imageInRight = nthRange(NESTED, '![diagram](img/diagram.png)', 1);
  cli(h().owner, ['thread', 'new', doc.id, imageInRight, '-m', 'An image in the first of two cells.']);
  await openDoc(page, doc.id);

  // Both cells of the row carry its lines, so the row is the smallest unambiguous block.
  const tr = page.locator('.cd-doc tr[data-t="t1"]');
  await expect(tr).toHaveCount(1);
  await expect(tr).toHaveClass(/cd-hl-block/);
  // Not the other cell (the one the reviewer's example marked by mistake) and not the table.
  await expect(page.locator('.cd-doc td:has-text("note")')).not.toHaveClass(/cd-hl-block/);
  await expect(page.locator('.cd-doc table[data-l="9-11"]')).not.toHaveClass(/cd-hl-block/);
});

// spec: S10-12
test('a loose list item marks the paragraph, and a nested item marks the inner item', async ({ page }) => {
  const doc = seedDoc('web-hl-loose', LOOSE);
  cli(h().owner, ['thread', 'new', doc.id, nthRange(LOOSE, '![diagram](img/diagram.png)', 0), '-m', 'The first paragraph.']);
  cli(h().owner, ['thread', 'new', doc.id, nthRange(LOOSE, '![diagram](img/diagram.png)', 1), '-m', 'The inner item.']);
  await openDoc(page, doc.id);

  // A loose item: two paragraphs share the `li` lines; one holds this line, so the paragraph is marked.
  const p = page.locator('.cd-doc li[data-l="3-5"] p[data-l="3-3"]');
  await expect(p).toHaveClass(/cd-hl-block/);
  await expect(p).toHaveAttribute('data-t', 't1');
  await expect(page.locator('.cd-doc li[data-l="3-5"]')).not.toHaveClass(/cd-hl-block/);
  // A nested item: the inner `li`, not the outer item or either list.
  const inner = page.locator('.cd-doc li[data-l="8-8"]');
  await expect(inner).toHaveClass(/cd-hl-block/);
  await expect(inner).toHaveAttribute('data-t', 't2');
  await expect(page.locator('.cd-doc li[data-l="7-8"]')).not.toHaveClass(/cd-hl-block/);
  expect(await page.locator('.cd-doc ul.cd-hl-block').count()).toBe(0);
});

// spec: S10-12
test('a plain form that occurs more than once on its line falls back to the block, not to a wrong word', async ({ page }) => {
  const md = `# Ambiguous\n\nThis is **is** key\n`;
  const doc = seedDoc('web-hl-ambiguous', md);
  cli(h().owner, ['thread', 'new', doc.id, '3:8-3:14', '-m', 'A short bold word.']);
  await openDoc(page, doc.id);

  // `is` occurs inside "This", as the word "is", and as the bold `is`: the server's `occ` counts
  // the raw quote, so the plain form cannot pick one safely. The paragraph is marked instead.
  const p = page.locator('.cd-doc p[data-l="3-3"]');
  await expect(p).toHaveClass(/cd-hl-block/);
  await expect(p).toHaveAttribute('data-t', 't1');
  await expect(page.locator('.cd-doc mark.cd-hl[data-t="t1"]')).toHaveCount(0);
});

// spec: S10-12
test('a quote with backslashes is found: inside a code span and around an escaped marker', async ({ page }) => {
  const doc = seedDoc('web-hl-escapes', ESCAPES);
  cli(h().owner, ['thread', 'new', doc.id, rangeOf(ESCAPES, '`a\\*b`'), '-m', 'A code span with a backslash.']);
  cli(h().owner, ['thread', 'new', doc.id, rangeOf(ESCAPES, 'a \\*b\\* c'), '-m', 'Escaped markers.']);
  await openDoc(page, doc.id);

  // Inside a code span a backslash is literal: the plain form keeps it.
  await expect(page.locator('.cd-doc mark.cd-hl[data-t="t1"]')).toHaveText('a\\*b');
  // An escaped marker is the character alone: the emphasis pass must not eat it as syntax.
  await expect(page.locator('.cd-doc mark.cd-hl[data-t="t2"]')).toHaveText('a *b* c');
  expect(await page.locator('.cd-doc .cd-hl-block').count()).toBe(0);
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
