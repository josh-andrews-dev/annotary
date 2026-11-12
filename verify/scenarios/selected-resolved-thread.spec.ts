// A Thread the person has selected stays on screen, marked resolved, when another Member resolves
// it over the live feed with "Show resolved" off; Escape hides it, like any other resolved Thread.
// On the copy the other Member is a fresh Member with its own Token, made the way
// `member-avatars.spec.ts` makes one: the owner mints a Token and POSTs `/api/v1/members`.
//
// What it creates: one Token, one Member and one span Thread on a real Document. At the end it
// tombstones the Thread's root Comment, so the Thread is fully tombstoned and no list or page shows
// it; the Member and Token, like every scenario's, live only on the copy `make verify-live` deletes.
import * as fs from 'node:fs';
import * as path from 'node:path';
import { test } from '@playwright/test';
import { URL, cli, cliJson, documents, expect, openDoc, shot } from '../lib';

/** The first plain paragraph of a Document (1-based line number, its text), skipping what the
 *  page does not render as a paragraph of its own: YAML front matter, fenced code, an HTML block,
 *  a setext heading, and list items including a lazy continuation line (one that starts at the
 *  margin but belongs to the item above). */
function firstParagraph(id: string): { n: number; text: string } | null {
  const dir = fs.mkdtempSync(path.join(process.env.VERIFY_TMP!, 'pull-'));
  cli(['pull', id, '--out', dir]);
  const md = fs.readdirSync(dir).find((f) => f.endsWith('.md'))!;
  const lines = fs.readFileSync(path.join(dir, md), 'utf8').split('\n')
    .map((line) => line.replace(/^\uFEFF/, '').replace(/\r$/, ''));
  // Front matter is parsed out before rendering; a closer may be `---` or `...`.
  let start = 0;
  if (lines[0] === '---') {
    const close = lines.findIndex((line, i) => i > 0 && (line === '---' || line === '...'));
    if (close > 0) start = close + 1;
  }
  let fence: string | null = null;
  let inList = false;
  // An HTML block ends at its comment close, else at the next blank line (CommonMark's block tags).
  let html: 'blank' | 'comment' | null = null;
  for (let i = start; i < lines.length; i++) {
    const line = lines[i];
    const f = /^\s{0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
      continue;
    }
    if (f) { fence = f[1]; inList = false; html = null; continue; }
    if (html) {
      if (html === 'comment' ? /-->/.test(line) : !line.trim()) html = null;
      continue;
    }
    if (/^\s{0,3}<!--/.test(line)) { if (!/-->/.test(line)) html = 'comment'; continue; }
    if (/^\s{0,3}</.test(line)) { html = 'blank'; continue; }
    // A blank line ends a list; a lazy continuation has none.
    if (!line.trim()) { inList = false; continue; }
    const item = /^\s*(?:[-*+]|\d+[.)])\s+\S/.test(line);
    if (inList) {
      // A new block (heading, quote, table, HTML, rule) ends the list; anything else is its lazy
      // continuation and is not a paragraph of its own.
      if (!item && !/^\s{0,3}(?:#{1,6}\s|>|\||<|(?:-{3,}|\*{3,}|_{3,})\s*$)/.test(line)) continue;
      inList = false;
    }
    if (item) { inList = true; continue; }
    if (/^(?: {4}|\t)/.test(line)) continue; // indented code
    // A text line followed by an underline is a setext heading, not a paragraph.
    if (/^\s{0,3}(?:=+|-+)\s*$/.test(lines[i + 1] ?? '')) continue;
    // A plain paragraph: letters at the start, no heading, list marker, quote, code or table.
    if (/^[A-Za-z][^`*_[\]<>|#-]{19,}$/.test(line)) return { n: i + 1, text: line };
  }
  return null;
}

function openThreads(id: string): number {
  const rows = cliJson<{ id: string }[]>(['thread', 'list', id]) ?? [];
  return rows.length;
}

test('a Thread another Member resolves while selected stays on screen, marked resolved, and hides when the person moves on', async ({ page }) => {
  const owner = cliJson<{ token: string }>(['token', 'mint', '--name', 'verify-resolved-selection']).token;
  const call = async (method: string, route: string, token: string, body?: unknown) => {
    const r = await fetch(URL + route, {
      method,
      headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    return { status: r.status, json: await r.clone().json().catch(() => null) };
  };
  const handle = `verify-resolved-${Date.now().toString(36)}`;
  const added = await call('POST', '/api/v1/members', owner, { handle, name: 'Verify Resolved' });
  expect(added.status).toBe(201);
  const token = added.json.token as string;
  expect(token).toBeTruthy();

  // A real Document with a paragraph to anchor on.
  const found = documents()
    .map((d) => ({ doc: d, p: firstParagraph(d.id) }))
    .find((x) => x.p !== null);
  expect(found, 'the copy has a Document with a plain paragraph').toBeTruthy();
  const { doc, p } = found!;
  test.info().annotations.push({ type: 'data', description: `Document ${doc.id}, line ${p!.n}` });

  const before = openThreads(doc.id);
  // The first few words, so the kept highlight is a phrase rather than one short word.
  const words = p!.text.split(/\s+/).slice(0, 4);
  const phrase = p!.text.slice(0, p!.text.indexOf(words[words.length - 1]) + words[words.length - 1].length);
  const tid = String(cliJson(['thread', 'new', doc.id, `${p!.n}:0-${p!.n}:${Array.from(phrase).length}`, '-m', 'A selected Thread another Member resolves.'])['id']);
  expect(tid).toMatch(/^t\d+$/);
  const openAfterNew = openThreads(doc.id);
  expect(openAfterNew).toBe(before + 1);

  await openDoc(page, doc.id);
  await page.waitForSelector('.cd-app.cd-live');
  const toggle = page.locator('#cd-resolved');
  if (await toggle.isChecked()) await toggle.uncheck();
  const card = page.locator(`.cd-thread[data-t="${tid}"]`);
  const mark = page.locator(`mark.cd-hl[data-t="${tid}"]`).first();
  // Select it the way a person does: by its text.
  await mark.click();
  await expect(card).toHaveClass(/cd-selected/);
  const resBefore = +(await page.locator('.cd-res-n').first().textContent()) || 0;

  // Another Member resolves it: a live event, not this browser's own mutation.
  const resolved = await call('POST', `/api/v1/docs/${doc.id}/threads/${tid}/resolve`, token);
  expect(resolved.status).toBe(200);

  await expect(card).toHaveClass(/cd-resolved/);
  await expect(card).toBeVisible();
  await expect(card).toHaveClass(/cd-selected/);
  await expect(card.locator('.cd-resline')).toContainText('resolved by Verify Resolved');
  await expect(mark).toHaveClass(/cd-selected/);
  const ink = await mark.evaluate((e) => getComputedStyle(e).backgroundColor);
  expect(ink, 'the selected resolved highlight is still drawn').not.toBe('rgba(0, 0, 0, 0)');
  await expect(toggle).not.toBeChecked();
  // The counts are of the Threads, not of what is on screen.
  await expect(page.locator('.cd-status').first()).toContainText(`${openAfterNew - 1} open`);
  await expect(page.locator('.cd-res-n').first()).toHaveText(String(resBefore + 1));
  await shot(page, 'resolved-selection-kept');

  // Moving on (Escape) hides it like any other resolved Thread, highlight and all.
  await page.keyboard.press('Escape');
  await expect(card).toBeHidden();
  await expect(page.locator('.cd-thread.cd-selected')).toHaveCount(0);
  const hiddenInk = await mark.evaluate((e) => getComputedStyle(e).backgroundColor);
  expect(hiddenInk).toBe('rgba(0, 0, 0, 0)');
  await shot(page, 'resolved-selection-hidden');

  // Cleanup: tombstone the root Comment, so the created Thread shows nowhere on the copy.
  cli(['thread', 'delete', doc.id, tid]);
});
