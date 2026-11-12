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
  // Leading YAML front matter is not the body: a `- tag` line in it renders no list item.
  let start = 0;
  if (lines[0] === '---') {
    const close = lines.indexOf('---', 1);
    if (close > 0) start = close + 1;
  }
  const items: Item[] = [];
  // A `-` line inside a fenced code block renders as code, not as a list item.
  let fence: string | null = null;
  lines.slice(start).forEach((line, k) => {
    const i = k + start;
    const f = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
      return;
    }
    if (f) { fence = f[1]; return; }
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
  // A tight or nested item marks its `li`; a loose item's paragraph is a child block, so accept
  // either the item itself or a mark or block inside it.
  const marked = await li.evaluate((e, t) => {
    if (e.getAttribute('data-t') === t && e.classList.contains('cd-hl-block')) return true;
    return !!e.querySelector(`mark.cd-hl[data-t="${t}"], .cd-hl-block[data-t="${t}"]`);
  }, tid);
  expect(marked, 'the item or a mark inside it carries this Thread').toBe(true);
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

  // The bullet marker never renders, so this quote cannot be found and the smallest block decides:
  // the item itself, or a child block of a loose item.
  const forced = items[0];
  const forcedTid = anchor(forced, 0);
  await openDoc(page, forced.id);
  const li = page.locator(`.cd-doc li[data-l^="${forced.n}-"]`).first();
  const block = await li.evaluate((e, t) => (
    e.getAttribute('data-t') === t ? e.classList.contains('cd-hl-block') : !!e.querySelector(`.cd-hl-block[data-t="${t}"]`)
  ), forcedTid);
  expect(block, 'the whole-item range marks a block at the item').toBe(true);
  await expectItemMarked(page, forced, forcedTid);
});
