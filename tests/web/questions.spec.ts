// Questions in a browser: the controls the server spliced into the first paint, answered with a
// click or the keyboard (optimistic, rolled back on failure), the live feed updating another
// viewer, Accept recommended, Other… opening the Question's Thread, a mid-review Question's card,
// and the read-only and narrow / dark states. The embed path is in `embed.spec.ts`; the server
// side in `crates/annotary/tests/questions_web.rs`.
import { test, expect, type Page } from '@playwright/test';
import { harness, cli, seedDoc, pushVersion } from './harness';
import { openDoc, waitLive } from './actions';

const MD = `# Launch plan

| Decision | Options | Recommended | Decided |
|---|---|---|---|
| D1 | A, B | A | |
| D2 | A, B, C | B | |

## D1. Where the service runs

- Recommended: A. The existing box.
- B. A new VM.

## D2. Which region

- A. us-east
- B. eu-west
- C. us-west
`;

const row = (page: Page, key: string) => page.locator(`.cd-qrow[data-q="${key}"]`);
const opt = (page: Page, key: string, o: string) => row(page, key).locator(`.cd-qopt[data-o="${o}"]`);
const answers = (id: string) => cli(harness().owner, ['show', id]).split('\n').find((l) => l.startsWith('questions:'));

// spec: S50-11
test('a click answers in place, the radio and the table follow, a failure rolls back', async ({ page }) => {
  const { id } = seedDoc('q-click', MD);
  await openDoc(page, id);
  await expect(page.locator('#cd-qsum')).toHaveText('0 of 2 answered');
  await expect(page.locator('#cd-qaccept')).toHaveText('Accept recommended (2)');
  await opt(page, 'D1', 'B').click();
  await expect(opt(page, 'D1', 'B')).toHaveAttribute('aria-pressed', 'true');
  await expect(row(page, 'D1').locator('.cd-qwho')).toHaveText('B @rowan');
  await expect(page.locator('input.cd-qradio[data-q="D1"][data-o="B"]')).toBeChecked();
  await expect(page.locator('.cd-qcell[data-q="D1"] .cd-qwho')).toHaveText('B @rowan');
  await expect(page.locator('#cd-qsum')).toHaveText('1 of 2 answered');
  await expect.poll(() => answers(id)).toBe('questions: D1=B D2=-');
  // The radio on an Option's own block answers too; the text beside it stays selectable.
  await page.locator('input.cd-qradio[data-q="D1"][data-o="A"]').check();
  await expect.poll(() => answers(id)).toBe('questions: D1=A D2=-');
  // The keyboard: a focused button answers on Space.
  await opt(page, 'D2', 'C').focus();
  await page.keyboard.press('Space');
  await expect.poll(() => answers(id)).toBe('questions: D1=A D2=C');
  // Pressed again: withdrawn.
  await opt(page, 'D2', 'C').click();
  await expect.poll(() => answers(id)).toBe('questions: D1=A D2=-');
  // A refused answer goes back, with the error.
  await page.route('**/questions/D2/answer', (r) => r.fulfill({ status: 500, body: '{"error":"internal","message":"boom"}' }));
  await opt(page, 'D2', 'A').click();
  await expect(page.locator('#cd-toast')).toContainText('Answer to D2 not saved');
  await expect(opt(page, 'D2', 'A')).toHaveAttribute('aria-pressed', 'false');
  await expect(row(page, 'D2').locator('.cd-qwho')).toHaveText('No answer yet');
});

// spec: S50-11
test('Accept recommended answers every open one; another viewer sees it live', async ({ page, browser }) => {
  const { id } = seedDoc('q-live', MD);
  const other = await browser.newPage();
  await other.setExtraHTTPHeaders({ Authorization: 'Bearer ' + harness().agent });
  await openDoc(other, id);
  await waitLive(other);
  await openDoc(page, id);
  await page.locator('#cd-qaccept').click();
  await expect.poll(() => answers(id)).toBe('questions: D1=A D2=B');
  await expect(page.locator('#cd-qaccept')).toBeHidden();
  await expect(other.locator('.cd-qrow[data-q="D2"] .cd-qwho')).toHaveText('B @rowan');
  await expect(other.locator('.cd-qrow[data-q="D2"] .cd-qopt[data-o="B"]')).toHaveAttribute('aria-pressed', 'false');
  await other.close();
});

// spec: S50-11, S50-12
test('a newer Version pushed while the page is open closes its controls; an answer names the Version shown', async ({ page }) => {
  const doc = seedDoc('q-stale', MD);
  await openDoc(page, doc.id);
  await waitLive(page);
  await expect(page.locator('#cd-qaccept')).toBeVisible();
  pushVersion(doc, MD.replace('- B. A new VM.', '- B. A new VM, bigger.'), 'v2');
  await expect(page.locator('#cd-newver')).toBeVisible();
  await expect(opt(page, 'D1', 'B')).toBeDisabled();
  await expect(page.locator('input.cd-qradio[data-q="D1"][data-o="B"]')).toBeDisabled();
  await expect(row(page, 'D1').locator('.cd-qcmt')).toBeDisabled();
  await expect(page.locator('#cd-qaccept')).toBeHidden();
  // Forced past the disabled state, the answer is for v1, which the server refuses.
  const sent = page.waitForRequest((r) => r.url().includes('/questions/D1/answer'));
  await opt(page, 'D1', 'B').evaluate((b: HTMLButtonElement) => { b.disabled = false; b.click(); });
  expect((await sent).postDataJSON()).toEqual({ option: 'B', version: 1 });
  await expect(page.locator('#cd-toast')).toContainText('Answer to D1 not saved');
  await expect(opt(page, 'D1', 'B')).toBeDisabled();
  expect(answers(doc.id)).toBe('questions: D1=- D2=-');
});

// spec: S50-09, S50-11
test('Other… opens the Question\'s Thread; its card leads with the answer', async ({ page }) => {
  const { id } = seedDoc('q-other', MD);
  await openDoc(page, id);
  await opt(page, 'D1', 'other').click();
  const composer = page.locator('.cd-composer');
  await expect(composer).toBeVisible();
  await composer.locator('textarea').fill('Neither: a managed service.');
  await page.keyboard.press('Control+Enter');
  const card = page.locator('.cd-thread[data-t="t1"]');
  await expect(card.locator('.cd-qans')).toContainText('Other @rowan');
  await expect.poll(() => answers(id)).toBe('questions: D1=other D2=-');
  // The Thread carries the key; the pull writes "Other: t1".
  expect(cli(harness().owner, ['thread', 'show', id, 't1'])).toContain('question: D1');
});

// spec: S50-12
test('a mid-review Question is answered from its card', async ({ page }) => {
  const { id } = seedDoc('q-ask', '# Plan\n\nWe ship on Friday.\n');
  cli(harness().agent, ['thread', 'new', id, '--document', '-m', 'Ship on Friday?', '--ask', 'Yes', '--ask', 'Wait a week']);
  await openDoc(page, id);
  const card = page.locator('.cd-thread[data-t="t1"]');
  await card.click();
  await card.locator('.cd-qopt[data-o="B"]').click();
  await expect(card.locator('.cd-qwho')).toHaveText('Wait a week @rowan');
  await expect.poll(() => answers(id)).toBe('questions: t1=B');
});

for (const scheme of ['light', 'dark'] as const) {
  // spec: S50-12
  test(`narrow and ${scheme}: the controls fit; an older Version is read-only; Compare has none`, async ({ page }) => {
    const doc = seedDoc(`q-states-${scheme}`, MD);
    cli(harness().owner, ['answer', doc.id, 'D1', 'B']);
    await page.emulateMedia({ colorScheme: scheme });
    await openDoc(page, doc.id, { width: 390 });
    const r = row(page, 'D1');
    await expect(r).toBeVisible();
    const fits = await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1);
    expect(fits).toBe(true);
    // Pressed is more than a color: the check mark shows.
    await expect(opt(page, 'D1', 'B').locator('.cd-qmark')).toBeVisible();
    await expect(opt(page, 'D1', 'A').locator('.cd-qmark')).toBeHidden();
    pushVersion(doc, MD.replace('| D1 | A, B | A | |', '| D1 | A, B | A | B (@rowan) |'), 'record');
    await openDoc(page, doc.id, { v: 1 });
    await expect(opt(page, 'D1', 'B')).toBeDisabled();
    await expect(opt(page, 'D1', 'B')).toHaveAttribute('aria-pressed', 'true');
    // The head: D1 decided, its row says so with no buttons.
    await openDoc(page, doc.id);
    await expect(row(page, 'D1')).toHaveClass(/cd-qdone/);
    await expect(row(page, 'D1').locator('.cd-qopt')).toHaveCount(0);
    await page.goto(`${harness().url}/d/${doc.id}/diff?from=1&to=2`);
    await expect(page.locator('.cd-qx')).toHaveCount(0);
  });
}

const DECIDED = MD.replace('| D1 | A, B | A | |', '| D1 | A, B | A | B (@rowan, 2026-11-20) |')
  .replace('| D2 | A, B, C | B | |', '| D2 | A, B, C | B | C (@rowan, 2026-11-20) |');

/** What a reader takes away from `#cd-doc`: everything selected and copied (no whitespace). */
const copiedText = (page: Page) =>
  page.evaluate(() => {
    const s = window.getSelection()!;
    s.selectAllChildren(document.getElementById('cd-doc')!);
    const t = s.toString().replace(/\s+/g, '');
    s.removeAllRanges();
    return t;
  });

// spec: S50-16
test('a Version with Questions reads back as its Compare page: the controls are not the Document\'s text', async ({ page }) => {
  const doc = seedDoc('q-compare-text', MD);
  pushVersion(doc, DECIDED, 'record');
  await openDoc(page, doc.id);
  await expect(row(page, 'D1')).toHaveClass(/cd-qdone/);
  // The convention every Document-text reader follows: `#cd-doc` without strips and Question chrome.
  const docText = await page.locator('#cd-doc').evaluate((root) => {
    const c = root.cloneNode(true) as HTMLElement;
    c.querySelectorAll('.cd-strip, .cd-qx').forEach((e) => e.remove());
    return (c.textContent || '').replace(/\s+/g, '');
  });
  expect(docText).not.toContain('decided:');
  // Selected and copied, the page gives the same text: the decided lines are not in it.
  expect(await copiedText(page)).toBe(docText);
  await page.goto(`${harness().url}/d/${doc.id}/diff?from=1&to=2`);
  await page.waitForSelector('.cd-app.cd-diffpage.cd-ready');
  const newer = await page.locator('#cd-dxdoc').evaluate((root) => {
    const c = root.cloneNode(true) as HTMLElement;
    c.querySelectorAll('del.cd-del, .cd-dx-del, .cd-sr, .cd-dx-sum, #cd-dxnote').forEach((e) => e.remove());
    return (c.textContent || '').replace(/\s+/g, '');
  });
  expect(newer).toBe(docText);
  // An open Question's buttons are not copied either.
  const open = seedDoc('q-copy-open', MD);
  await openDoc(page, open.id);
  const copied = await copiedText(page);
  expect(copied).not.toContain('Other…');
  expect(copied).not.toContain('Noansweryet');
});

for (const [state, width] of [['open', 1280], ['decided', 1280], ['open', 390]] as const) {
  // spec: S50-16
  test(`a selection across a Question's controls (${state}, ${width} px) maps to the source it covers`, async ({ page }) => {
    const md = state === 'open' ? MD : DECIDED;
    const doc = seedDoc(`q-select-${state}-${width}`, md);
    await openDoc(page, doc.id, { width });
    await expect(row(page, 'D1')).toBeVisible();
    // From the place heading, over its row (and the Option's radio when open), into the Option.
    await page.evaluate(() => {
      const textIn = (sel: string, t: string) => {
        const w = document.createTreeWalker(document.querySelector(sel)!, NodeFilter.SHOW_TEXT);
        for (let n = w.nextNode(); n; n = w.nextNode()) {
          const k = (n.nodeValue || '').indexOf(t);
          if (k >= 0 && !n.parentElement!.closest('.cd-qx')) return { n, k };
        }
        throw new Error(`"${t}" not in ${sel}`);
      };
      const a = textIn('#cd-doc h2', 'Where the service runs');
      const b = textIn('#cd-doc li', 'The existing box');
      const r = document.createRange();
      r.setStart(a.n, a.k);
      r.setEnd(b.n, b.k + 'The existing box'.length);
      const s = window.getSelection()!;
      s.removeAllRanges();
      s.addRange(r);
    });
    const pill = page.locator('#cd-pill');
    await expect(pill).toBeVisible();
    await pill.click();
    const exact = 'Where the service runs\n\n- Recommended: A. The existing box';
    const quote = page.locator('.cd-composer .cd-quote q');
    await expect(quote).toContainText('Where the service runs');
    await expect(quote).not.toContainText('decided');
    await expect(quote).not.toContainText('Other');
    const ta = page.locator('.cd-composer textarea');
    await ta.fill('Across the controls.');
    await page.keyboard.press('Control+Enter');
    await expect(page.locator('.cd-thread[data-t="t1"]:not(.cd-pending)')).toBeVisible();
    const t1 = JSON.parse(cli(harness().owner, ['thread', 'show', doc.id, 't1', '--json']))[0];
    const lines = md.split('\n');
    const l0 = lines.findIndex((l) => l.includes('Where the service runs'));
    const l1 = lines.findIndex((l) => l.includes('The existing box'));
    const c0 = lines[l0].indexOf('Where');
    const c1 = lines[l1].indexOf('The existing box') + 'The existing box'.length;
    expect(t1.anchor.original.range).toBe(`${l0 + 1}:${c0}-${l1 + 1}:${c1}`);
    expect(t1.anchor.original.exact).toBe(exact);
  });
}

/** Selects from the start of the first text node under `from` to the end of `to`'s text `t`. */
const selectFrom = (page: Page, from: string, to: string, t: string) =>
  page.evaluate(([from, to, t]) => {
    const first = document.createTreeWalker(document.querySelector(from)!, NodeFilter.SHOW_TEXT).nextNode()!;
    const w = document.createTreeWalker(document.querySelector(to)!, NodeFilter.SHOW_TEXT);
    let n = w.nextNode();
    while (n && (n.nodeValue || '').indexOf(t) < 0) n = w.nextNode();
    const r = document.createRange();
    r.setStart(first, 0);
    r.setEnd(n!, (n!.nodeValue || '').indexOf(t) + t.length);
    const s = window.getSelection()!;
    s.removeAllRanges();
    s.addRange(r);
  }, [from, to, t]);

for (const [from, to, t, quote] of [
  ['.cd-qrow[data-q="D1"] .cd-qwho', '#cd-doc li', 'The existing box', 'Recommended: A. The existing box'],
  ['.cd-qcell[data-q="D1"] .cd-qwho', '#cd-doc tbody tr:nth-child(2) td', 'D2', 'D2'],
] as const) {
  // spec: S50-16
  test(`a selection that starts in ${from} maps from the Document text after it`, async ({ page }) => {
    const doc = seedDoc(`q-start-${from.includes('qrow') ? 'row' : 'cell'}`, MD);
    await openDoc(page, doc.id);
    await expect(row(page, 'D1')).toBeVisible();
    await selectFrom(page, from, to, t);
    await expect(page.locator('#cd-pill')).toBeVisible();
    await page.locator('#cd-pill').click();
    await expect(page.locator('.cd-composer .cd-quote q')).toHaveText(quote);
    await expect(page.locator('#cd-toast')).not.toContainText('Could not map');
  });
}

// spec: S50-16
test('a selection wholly inside a Question\'s controls offers no comment', async ({ page }) => {
  const doc = seedDoc('q-inside', MD);
  await openDoc(page, doc.id);
  await expect(row(page, 'D1')).toBeVisible();
  for (const sel of ['.cd-qcell[data-q="D1"]', '.cd-qrow[data-q="D1"] .cd-qwho']) {
    // Document text first, so the pill is up and the Question's selection has to take it down.
    await selectFrom(page, '#cd-doc h2', '#cd-doc h2', 'Where the service runs');
    await expect(page.locator('#cd-pill')).toBeVisible();
    await page.evaluate((sel) => {
      const r = document.createRange();
      r.selectNodeContents(document.querySelector(sel)!);
      const s = window.getSelection()!;
      s.removeAllRanges();
      s.addRange(r);
    }, sel);
    await expect(page.locator('#cd-pill')).toBeHidden();
  }
});
