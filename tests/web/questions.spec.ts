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
