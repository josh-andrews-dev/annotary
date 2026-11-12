// A comment anchored inside one list item on the real data marks that item — or the quoted words
// in it — never the whole list, and its margin card aligns with the item. Two anchors are made on
// a list of the copy: one whose quote carries inline Markdown syntax (found in its plain form when
// the item has any), and one whose range starts at the bullet marker, which never renders, so the
// smallest-block rule decides. The copy is thrown away.
import * as fs from 'node:fs';
import * as path from 'node:path';
import { test } from '@playwright/test';
import { cli, cliJson, documents, expect, openDoc, shot } from '../lib';

interface Item {
  id: string;
  n: number;
  line: string;
}

/** The list items of a Document's head Markdown (`n` = 1-based source line). */
function listItems(doc: { id: string }): Item[] {
  const dir = fs.mkdtempSync(path.join(process.env.VERIFY_TMP!, 'pull-'));
  cli(['pull', doc.id, '--out', dir]);
  const md = fs.readdirSync(dir).find((f) => f.endsWith('.md'))!;
  const lines = fs.readFileSync(path.join(dir, md), 'utf8').split('\n');
  const items: Item[] = [];
  lines.forEach((line, i) => {
    if (/^\s*(?:[-*+]|\d+\.)\s+\S/.test(line)) items.push({ id: doc.id, n: i + 1, line });
  });
  return items;
}

/** A Thread on a range of a list item; returns its id. */
function anchor(item: Item, startCol: number): string {
  const end = Array.from(item.line).length;
  return String(cliJson(['thread', 'new', item.id, `${item.n}:${startCol}-${item.n}:${end}`, '-m', 'Checks the anchor inside one list item.', '--json', 'id'])['id']);
}

/** The item's mark is the Thread's, the list around it is not, and the card sits at the item. */
async function expectItemMarked(page: import('@playwright/test').Page, item: Item, tid: string) {
  const li = page.locator(`.cd-doc li[data-l^="${item.n}-"]`).first();
  await expect(li).toHaveCount(1);
  const mark = li.locator(`mark.cd-hl[data-t="${tid}"]`).first();
  const block = await li.evaluate((e, t) => e.classList.contains('cd-hl-block') && e.getAttribute('data-t') === t, tid);
  if (block) await expect(li).toHaveClass(/cd-hl-block/);
  else await expect(mark).toHaveCount(1);
  // No list around it carries this Thread: no bar beside every bullet, no list-wide block mark.
  const lists = page.locator('.cd-doc ul, .cd-doc ol');
  expect(await lists.evaluateAll((els, t) => els.filter((e) => e.getAttribute('data-t') === t).length, tid)).toBe(0);
  // Layout A: the card aligns with the item, not with the top of the list.
  const strip = page.locator('.cd-strip').filter({ has: page.locator(`.cd-thread[data-t="${tid}"]`) }).first();
  const lb = (await li.boundingBox())!;
  const sb = (await strip.boundingBox())!;
  expect(sb.y, 'the card starts at the item, never above it').toBeGreaterThanOrEqual(lb.y - 2);
  // Cards above it in the margin may push it down (`layout()`), so only the start is bounded.
  expect(sb.y - lb.y, 'the card sits at the item, not at the top of the list').toBeLessThan(160);
  await shot(page, `list-item-highlight-${item.id}-${item.n}`);
}

test('a comment inside one list item marks that item on the real data, never the whole list', async ({ page }) => {
  let items: Item[] = [];
  for (const d of documents()) {
    items = listItems(d);
    if (items.length) break;
  }
  expect(items.length, 'the live data has a Document with a Markdown list').toBeGreaterThan(0);

  // The quote with inline Markdown syntax: the item's text from after its marker, so the frozen
  // exact carries the syntax (`**`, a code span, a link) the rendered text does not.
  const syntax = items.find((i, k) => k > 0 && /\*\*|`|\[[^\]]*\]\(/.test(i.line.replace(/^\s*(?:[-*+]|\d+\.)\s+/, '')));
  test.info().annotations.push({ type: 'data', description: syntax ? `inline syntax: ${syntax.id}:${syntax.n}` : 'no list item with inline syntax in the copy' });
  if (syntax) {
    const prefix = /^\s*(?:[-*+]|\d+\.)\s+/.exec(syntax.line)![0];
    const tid = anchor(syntax, Array.from(prefix).length);
    await openDoc(page, syntax.id);
    await expectItemMarked(page, syntax, tid);
  }

  // The bullet marker never renders, so this quote cannot be found and the smallest block decides.
  const forced = items[0];
  const forcedTid = anchor(forced, 0);
  await openDoc(page, forced.id);
  const li = page.locator(`.cd-doc li[data-l^="${forced.n}-"]`).first();
  await expect(li).toHaveClass(/cd-hl-block/);
  await expect(li).toHaveAttribute('data-t', forcedTid);
  await expectItemMarked(page, forced, forcedTid);
});
