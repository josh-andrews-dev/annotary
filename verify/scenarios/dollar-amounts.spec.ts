// S44-01 on the real data: dollar amounts in prose render as text, not inline math, in the
// Documents of the copy (rendered by an older renderer, so the page re-renders them on read) and
// in a Comment posted on a real Document. Finds the lines by their Markdown, never by an id.
import * as fs from 'node:fs';
import * as path from 'node:path';
import { test, type Page } from '@playwright/test';
import { cli, cliJson, documents, expect, openDoc, shot } from '../lib';

// An opening `$` before a digit, then a closing `$` after a non-space and before a digit: what
// pulldown-cmark alone pairs as math. Lines with inline markup or escapes are left out, so the
// match is also the text the page shows.
const PAIR = /\$\d[^$\n]*?\S\$\d[\d.,]*/;
const MARKUP = /[*_`[\]<>\\{}^]/;

const SENTENCE =
  'Your ingredient cost per pack is maybe $1.00–1.25 (a 24-pack batch is 72 cookies out of a ~$120 shop), so...';

const flat = (s: string) => s.replace(/\s+/g, ' ').trim();

/** The text of the innermost block whose `data-l` holds source line `line`. */
async function blockText(page: Page, line: number): Promise<{ text: string; math: number } | null> {
  return page.evaluate((line) => {
    let best: Element | null = null;
    let span = Infinity;
    for (const el of document.querySelectorAll('.cd-doc [data-l]')) {
      const [a, b] = (el.getAttribute('data-l') || '').split('-').map(Number);
      if (a <= line && line <= b && b - a <= span && !el.matches('ul,ol,blockquote,table,thead,tbody,section')) {
        best = el;
        span = b - a;
      }
    }
    if (!best) return null;
    return { text: best.textContent || '', math: best.querySelectorAll('math').length };
  }, line);
}

test('dollar amounts in real Documents and Comments render as text', async ({ page }) => {
  const real = documents().filter((d) => !d.name.startsWith('verify-live'));
  expect(real.length).toBeGreaterThan(0);
  const tmp = fs.mkdtempSync(path.join(process.env.VERIFY_TMP!, 'dollars-'));

  // Documents: every real line with such a pair (at most 20) shows the pair as written.
  const hits: { id: string; line: number; text: string }[] = [];
  for (const d of real) {
    if (hits.length >= 20) break;
    const dir = fs.mkdtempSync(path.join(tmp, 'pull-'));
    cli(['pull', d.id, '--out', dir]);
    const md = fs.readdirSync(dir).find((f) => f.endsWith('.md'));
    if (!md) continue;
    const lines = fs.readFileSync(path.join(dir, md), 'utf8').split('\n');
    // Front matter is never rendered: start after it.
    const body = lines[0]?.trim() === '---' ? lines.findIndex((l, i) => i > 0 && /^(---|\.\.\.)\s*$/.test(l)) + 1 : 0;
    let fence = false;
    lines.forEach((l, i) => {
      if (i < body) return;
      if (/^\s*(```|~~~)/.test(l)) fence = !fence;
      const m = PAIR.exec(l);
      if (!fence && m && !MARKUP.test(l) && !/^\s*(\$\$|    )/.test(l)) hits.push({ id: d.id, line: i + 1, text: m[0] });
    });
  }
  let shown = false;
  for (const h of hits) {
    await openDoc(page, h.id);
    const block = await blockText(page, h.line);
    expect(block, `${h.id} line ${h.line}`).not.toBeNull();
    expect(flat(block!.text), `${h.id} line ${h.line}`).toContain(flat(h.text));
    if (!shown) {
      await page.locator(`.cd-doc [data-l^="${h.line}-"]`).first().scrollIntoViewIfNeeded();
      await shot(page, 'dollar-amounts-document');
      shown = true;
    }
  }

  // Comments: a real Comment with such a pair, and one posted now on a real Document.
  const d = real[0];
  const posted = JSON.parse(cli(['thread', 'new', d.id, '--document', '-m', SENTENCE, '--json', 'id'])).id;
  let comments = 0;
  for (const doc of real.slice(0, 30)) {
    const threads: any[] = cliJson(['thread', 'list', doc.id, '--all']) ?? [];
    const t = threads.find((t) => t.id !== posted && PAIR.test(t.body ?? '') && !MARKUP.test(t.body ?? ''));
    if (!t) continue;
    await openDoc(page, doc.id);
    const card = page.locator(`.cd-thread[data-t="${t.id}"]`).first();
    if (!(await card.isVisible())) continue;
    await expect(card.locator('math')).toHaveCount(0);
    expect(flat((await card.textContent()) ?? '')).toContain(flat(PAIR.exec(t.body)![0]));
    comments++;
    break;
  }
  await openDoc(page, d.id);
  const card = page.locator(`.cd-thread[data-t="${posted}"]`).first();
  await expect(card).toContainText('$1.00–1.25 (a 24-pack batch is 72 cookies out of a ~$120 shop)');
  await expect(card.locator('math')).toHaveCount(0);
  await card.scrollIntoViewIfNeeded();
  await shot(page, 'dollar-amounts-comment');

  const data = `${hits.length} Document lines with dollar pairs; ${comments} existing Comment; posted ${posted} on ${d.id}`;
  test.info().annotations.push({ type: 'data', description: data });
  console.log(`S44-01 data: ${data}`); // the report is deleted with the copy; the log keeps this
});
