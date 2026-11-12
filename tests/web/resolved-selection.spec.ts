// A selected Thread stays on screen, marked resolved, when it becomes resolved with "Show
// resolved" off; it hides when the person moves on. Another Member resolves it through the real
// CLI (as @scribe) so the change arrives as a live event; the cases cover a span, a block, an
// Asset and the whole Document, keyboard and pointer deselection, and a reply half typed.
import * as fs from 'node:fs';
import * as path from 'node:path';
import { test, expect, type Page } from '@playwright/test';
import { harness, cli, seedDoc, rangeOf, REPO_ROOT } from './harness';
import { openDoc, waitLive } from './actions';

const h = () => harness();

const MD = `# Stays

Alpha paragraph with words to anchor a Thread on.

Beta paragraph for a second one.

Gamma paragraph at the end.
`;

const PNG = fs.readFileSync(path.join(REPO_ROOT, 'tests', 'e2e', '48-reanchor-seed', 'seed.in', 'img', 'architecture.png'));

const card = (page: Page, tid: string) => page.locator(`.cd-thread[data-t="${tid}"]`);
const mark = (page: Page, tid: string) => page.locator(`mark.cd-hl[data-t="${tid}"]`).first();

async function newThreads(docId: string, md: string): Promise<string[]> {
  cli(h().agent, ['thread', 'new', docId, rangeOf(md, 'Alpha paragraph'), '-m', 'Alpha?']);
  cli(h().agent, ['thread', 'new', docId, rangeOf(md, 'Beta paragraph'), '-m', 'Beta?']);
  cli(h().agent, ['thread', 'new', docId, rangeOf(md, 'Gamma paragraph'), '-m', 'Gamma?']);
  return ['t1', 't2', 't3'];
}

function background(page: Page, selector: string) {
  return page.locator(selector).first().evaluate((e) => getComputedStyle(e).backgroundColor);
}

test.describe('a selected Thread stays when it is resolved', () => {
  // spec: S47-01, S47-02
  test('another Member resolves the selected Thread: it stays with its highlight, marked resolved, and hides when another is selected', async ({ page }) => {
    const doc = seedDoc('web-r1', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id);
    await waitLive(page);
    const toggle = page.locator('#cd-resolved');
    await expect(toggle).not.toBeChecked();
    // Pointer selection: the Thread's text.
    await mark(page, 't1').click();
    await expect(card(page, 't1')).toHaveClass(/cd-selected/);
    cli(h().agent, ['thread', 'resolve', doc.id, 't1']);
    // Still there, resolved, with its highlight and its selected mark.
    const t1 = card(page, 't1');
    await expect(t1).toBeVisible();
    await expect(t1).toHaveClass(/cd-resolved/);
    await expect(t1).toHaveClass(/cd-selected/);
    await expect(t1.locator('.cd-resline')).toContainText('resolved by Scribe');
    await expect(mark(page, 't1')).toHaveClass(/cd-selected/);
    expect(await background(page, 'mark.cd-hl[data-t="t1"]')).not.toBe('rgba(0, 0, 0, 0)');
    await expect(toggle).not.toBeChecked();
    // The counts are of the data, not of what is shown.
    await expect(page.locator('.cd-status')).toContainText('2 open');
    await expect(page.locator('.cd-res-n')).toHaveText('1');
    // Pointer deselection: selecting another Thread hides the resolved one like any other.
    await mark(page, 't2').click();
    await expect(t1).toBeHidden();
    await expect(t1).not.toHaveClass(/cd-selected/);
    expect(await background(page, 'mark.cd-hl[data-t="t1"]')).toBe('rgba(0, 0, 0, 0)');
    await expect(page.locator('.cd-status')).toContainText('2 open');
    await expect(page.locator('.cd-res-n')).toHaveText('1');
  });

  // spec: S47-03
  test('a reply half typed survives a live resolve, and the card says resolved while the box stays', async ({ page }) => {
    const doc = seedDoc('web-r2', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id);
    await waitLive(page);
    const t1 = card(page, 't1');
    await mark(page, 't1').click();
    await t1.locator('[data-act="reply"]').click();
    const box = t1.locator('.cd-editor-reply textarea');
    await expect(box).toBeFocused();
    await box.pressSequentially('half a thought');
    cli(h().agent, ['thread', 'reply', doc.id, 't1', '--resolve', '-m', 'Done.']);
    // The resolve shows at once; the editor, its text and its focus stay.
    await expect(t1).toBeVisible();
    await expect(t1).toHaveClass(/cd-resolved/);
    await expect(t1.locator('.cd-resline')).toContainText('resolved by Scribe');
    await expect(box).toBeVisible();
    await expect(box).toBeFocused();
    await expect(box).toHaveValue('half a thought');
    await expect(t1.locator('.cd-c')).toHaveCount(1);
    await expect(page.locator('.cd-status')).toContainText('2 open');
    await expect(page.locator('.cd-res-n')).toHaveText('1');
    // Cancel re-draws the card from the server; it is still the selection, so it stays.
    await t1.locator('.cd-editor-reply [data-act="cancel"]').click();
    await expect(t1.locator('.cd-editor')).toHaveCount(0);
    await expect(t1).toBeVisible();
    await expect(t1).toHaveClass(/cd-resolved/);
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-cbody')).toContainText('Done.');
    // Escape deselects and hides it.
    await page.keyboard.press('Escape');
    await expect(t1).toBeHidden();
    await expect(page.locator('.cd-thread.cd-selected')).toHaveCount(0);
  });

  // spec: S47-01, S47-02
  test('span, block, Asset and whole-Document Threads all stay while selected and hide on Escape', async ({ page }) => {
    const md = `# Everywhere

Delta paragraph for the span.

- **Recommended: A.** Leave it running.

![architecture](img/architecture.png)
`;
    const doc = seedDoc('web-r3', md, { 'img/architecture.png': PNG });
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(md, 'Delta paragraph'), '-m', 'Span.']);
    // A block: the range starts at the bullet marker, which never renders, so the item is marked.
    const listLine = md.split('\n').findIndex((l) => l.startsWith('- ')) + 1;
    const listLen = Array.from(md.split('\n')[listLine - 1]).length;
    cli(h().agent, ['thread', 'new', doc.id, `${listLine}:0-${listLine}:${listLen}`, '-m', 'Block.']);
    cli(h().agent, ['thread', 'new', doc.id, '--asset', 'img/architecture.png', '-m', 'Asset.']);
    cli(h().agent, ['thread', 'new', doc.id, '--document', '-m', 'Whole Document.']);
    await openDoc(page, doc.id);
    await waitLive(page);
    const ids = ['t1', 't2', 't3', 't4'];
    const marks: Record<string, string> = { t1: 'mark.cd-hl[data-t="t1"]', t2: '.cd-hl-block[data-t="t2"]', t3: '.cd-asset-hl[data-t="t3"]' };
    for (const tid of ids) {
      // Select by pointer: the card's row (a block mark is not a click target of its own today).
      await card(page, tid).locator('.cd-row').click();
      await expect(card(page, tid)).toHaveClass(/cd-selected/);
      cli(h().agent, ['thread', 'resolve', doc.id, tid]);
      await expect(card(page, tid)).toBeVisible();
      await expect(card(page, tid)).toHaveClass(/cd-resolved/);
      const sel = marks[tid];
      if (sel) {
        await expect(page.locator(sel).first()).toHaveClass(/cd-selected/);
        if (sel.startsWith('mark')) expect(await background(page, sel)).not.toBe('rgba(0, 0, 0, 0)');
        if (sel.startsWith('.cd-hl-block')) {
          expect(await page.locator(sel).first().evaluate((e) => getComputedStyle(e).boxShadow)).not.toBe('none');
        }
      }
      await page.keyboard.press('Escape');
      await expect(card(page, tid)).toBeHidden();
      await expect(page.locator('.cd-thread.cd-selected')).toHaveCount(0);
    }
    // Nothing resolved is shown; the counts cover all four.
    await expect(page.locator('.cd-status')).toContainText('0 open');
    await expect(page.locator('.cd-res-n')).toHaveText('4');
    await expect(page.locator('.cd-app')).not.toHaveClass(/cd-show-resolved/);
  });

  // spec: S47-04
  test('keyboard: j moves off the still-shown resolved Thread and hides it; k skips it; focus never stays in a hidden card', async ({ page }) => {
    const doc = seedDoc('web-r4', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id);
    await waitLive(page);
    await page.keyboard.press('j');
    await expect(card(page, 't1')).toHaveClass(/cd-selected/);
    cli(h().agent, ['thread', 'resolve', doc.id, 't1']);
    await expect(card(page, 't1')).toHaveClass(/cd-resolved/);
    await expect(card(page, 't1')).toBeVisible();
    // j moves on: the resolved Thread hides and is not a stop any more.
    await page.keyboard.press('j');
    await expect(card(page, 't2')).toHaveClass(/cd-selected/);
    await expect(card(page, 't1')).toBeHidden();
    await page.keyboard.press('k');
    await expect(card(page, 't2')).toHaveClass(/cd-selected/);
    await expect(card(page, 't1')).toBeHidden();
    // The same for a resolve from the card: focus in the card never lands on hidden content.
    cli(h().agent, ['thread', 'resolve', doc.id, 't2']);
    await expect(card(page, 't2')).toHaveClass(/cd-resolved/);
    await expect(card(page, 't2')).toBeVisible();
    await card(page, 't2').locator('[data-act="reopen"]').focus();
    await page.keyboard.press('j');
    await expect(card(page, 't3')).toHaveClass(/cd-selected/);
    await expect(card(page, 't2')).toBeHidden();
    expect(await page.evaluate(() => !!document.activeElement && !!document.activeElement.closest?.('.cd-thread[data-t="t2"]'))).toBe(false);
    await page.keyboard.press('Escape');
    await expect(page.locator('.cd-thread.cd-selected')).toHaveCount(0);
  });

  // spec: S47-05
  test('Show resolved on changes nothing; a resolved Thread that was not selected hides at once; my own resolve keeps the selection until I move on', async ({ page }) => {
    const doc = seedDoc('web-r5', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id);
    await waitLive(page);
    // Not selected: it hides at once, as today.
    cli(h().agent, ['thread', 'resolve', doc.id, 't3']);
    await expect(card(page, 't3')).toBeHidden();
    await expect(page.locator('.cd-status')).toContainText('2 open');
    // Show resolved on: the resolved card is on screen, walking away leaves it there.
    await page.locator('#cd-resolved').check();
    await expect(card(page, 't3')).toBeVisible();
    await mark(page, 't1').click();
    cli(h().agent, ['thread', 'resolve', doc.id, 't1']);
    await expect(card(page, 't1')).toBeVisible();
    await expect(card(page, 't1')).toHaveClass(/cd-resolved/);
    await page.keyboard.press('Escape');
    await expect(card(page, 't1')).toBeVisible();
    await expect(card(page, 't1')).toHaveClass(/cd-resolved/);
    await expect(page.locator('.cd-res-n')).toHaveText('2');
    // My own resolve by `e` on the selection: it stays until I move on.
    await page.locator('#cd-resolved').uncheck();
    await mark(page, 't2').click();
    await page.keyboard.press('e');
    await expect(card(page, 't2')).toHaveClass(/cd-resolved/);
    await expect(card(page, 't2')).toBeVisible();
    await expect(page.locator('.cd-status')).toContainText('0 open');
    await expect(card(page, 't2').locator('.cd-resline')).toContainText('resolved by Rowan Vale');
    await page.keyboard.press('Escape');
    await expect(card(page, 't2')).toBeHidden();
  });

  // spec: S47-04
  test('a link to a resolved Thread keeps what it did: Show resolved turns on and the Thread is selected', async ({ page }) => {
    const doc = seedDoc('web-r6', MD);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(MD, 'Alpha paragraph'), '-m', 'Already closed.']);
    cli(h().agent, ['thread', 'resolve', doc.id, 't1']);
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto(`${h().url}/d/${doc.id}#t1`);
    await page.waitForSelector('.cd-app.cd-ready');
    await expect(page.locator('.cd-app')).toHaveClass(/cd-show-resolved/);
    await expect(page.locator('#cd-resolved')).toBeChecked();
    await expect(card(page, 't1')).toHaveClass(/cd-selected/);
    await expect(card(page, 't1')).toBeVisible();
  });

  // spec: S47-02
  test('narrow layout: a selected row resolved live stays open; tapping it again collapses and hides it, and focus leaves it', async ({ page }) => {
    const doc = seedDoc('web-r7', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id, { width: 800 });
    await waitLive(page);
    const t1 = card(page, 't1');
    await t1.locator('.cd-row').click();
    await expect(t1).toHaveClass(/cd-open/);
    cli(h().agent, ['thread', 'resolve', doc.id, 't1']);
    await expect(t1).toBeVisible();
    await expect(t1).toHaveClass(/cd-resolved/);
    await expect(t1.locator('.cd-body')).toBeVisible();
    await expect(page.locator('.cd-res-n')).toHaveText('1');
    // Focus inside the row's controls does not stay in it when the row collapses.
    await t1.locator('[data-act="reopen"]').focus();
    await t1.locator('.cd-row').dispatchEvent('click');
    await expect(t1).toBeHidden();
    await expect(page.locator('.cd-thread.cd-selected')).toHaveCount(0);
    expect(await page.evaluate(() => !!document.activeElement && !!document.activeElement.closest?.('.cd-thread[data-t="t1"]'))).toBe(false);
  });

  // spec: S47-02
  test('narrow layout: tapping a row that holds a typed reply keeps it open until the reply is canceled', async ({ page }) => {
    const doc = seedDoc('web-r8', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id, { width: 800 });
    await waitLive(page);
    const t1 = card(page, 't1');
    await t1.locator('.cd-row').click();
    cli(h().agent, ['thread', 'resolve', doc.id, 't1']);
    await expect(t1).toHaveClass(/cd-resolved/);
    await expect(t1).toBeVisible();
    await t1.locator('[data-act="reply"]').click();
    const box = t1.locator('.cd-editor-reply textarea');
    await box.fill('half a thought');
    await t1.locator('.cd-row').dispatchEvent('click');
    await expect(t1).toBeVisible();
    await expect(t1).toHaveClass(/cd-open/);
    await expect(t1).not.toHaveClass(/cd-selected/);
    await expect(box).toBeVisible();
    await expect(box).toHaveValue('half a thought');
    await t1.locator('.cd-editor-reply [data-act="cancel"]').click();
    await expect(t1.locator('.cd-editor')).toHaveCount(0);
    await expect(t1).toBeHidden();
  });

  // spec: S47-01, S47-02
  test('a typed reply keeps its card on screen when another Thread is selected, and Escape is never trapped in a hidden editor', async ({ page }) => {
    const doc = seedDoc('web-r9', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id);
    await waitLive(page);
    const t1 = card(page, 't1'), t2 = card(page, 't2');
    await mark(page, 't1').click();
    await t1.locator('[data-act="reply"]').click();
    const box = t1.locator('.cd-editor-reply textarea');
    await box.pressSequentially('half a thought');
    cli(h().agent, ['thread', 'resolve', doc.id, 't1']);
    await expect(t1).toHaveClass(/cd-resolved/);
    await expect(t1).toBeVisible();
    await expect(box).toBeFocused();
    // Show resolved on and off again while the editor is open: the card stays either way.
    await page.locator('#cd-resolved').check();
    await expect(t1).toBeVisible();
    await page.locator('#cd-resolved').uncheck();
    await expect(t1).toBeVisible();
    // Selecting another Thread does not take the typed text away with it, nor does `j` moving on,
    // nor opening a second reply on the newly selected Thread: both editors stay where they are.
    await mark(page, 't2').click();
    await expect(t2).toHaveClass(/cd-selected/);
    await expect(t1).toBeVisible();
    await expect(t1).not.toHaveClass(/cd-selected/);
    await expect(box).toBeVisible();
    await expect(box).toHaveValue('half a thought');
    await page.keyboard.press('j');
    const t3 = card(page, 't3');
    await expect(t3).toHaveClass(/cd-selected/);
    await expect(t1).toBeVisible();
    await expect(box).toBeVisible();
    await page.keyboard.press('r');
    await expect(t3.locator('.cd-editor-reply textarea')).toBeFocused();
    await expect(page.locator('.cd-editor-reply')).toHaveCount(2);
    await expect(t1.locator('.cd-editor-reply textarea')).toBeVisible();
    await t3.locator('.cd-editor-reply [data-act="cancel"]').click();
    await expect(page.locator('.cd-editor-reply')).toHaveCount(1);
    // Escape with the focus in the kept editor asks there — visibly — and keep gives it back.
    await box.focus();
    await page.keyboard.press('Escape');
    await expect(t1.locator('.cd-discard')).toBeVisible();
    await expect(t3).toHaveClass(/cd-selected/);
    await t1.locator('.cd-discard [data-act="discard-no"]').click();
    await expect(box).toBeFocused();
    // Discard: the editor closes, the card re-draws resolved and unselected, then hides.
    await page.keyboard.press('Escape');
    await t1.locator('.cd-discard [data-act="discard-yes"]').click();
    await expect(t1.locator('.cd-editor')).toHaveCount(0);
    await expect(t1).toBeHidden();
    await expect(t3).toHaveClass(/cd-selected/);
    // Escape with the focus outside the editor deselects the Thread.
    await page.locator('.cd-doc h1').click();
    await page.keyboard.press('Escape');
    await expect(page.locator('.cd-thread.cd-selected')).toHaveCount(0);
  });

  // spec: S47-01, S47-03
  test('sending a reply after a live resolve posts it and keeps the card on screen', async ({ page }) => {
    const doc = seedDoc('web-r14', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id);
    await waitLive(page);
    const t1 = card(page, 't1');
    await mark(page, 't1').click();
    await t1.locator('[data-act="reply"]').click();
    const box = t1.locator('.cd-editor-reply textarea');
    await box.fill('a thought I meant to send');
    cli(h().agent, ['thread', 'resolve', doc.id, 't1']);
    await expect(t1).toHaveClass(/cd-resolved/);
    await expect(box).toBeVisible();
    await page.keyboard.press('Enter');
    await expect(t1.locator('.cd-editor')).toHaveCount(0);
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-cbody')).toContainText('a thought I meant to send');
    await expect(t1).toHaveClass(/cd-resolved/);
    await expect(t1).toBeVisible();
    await expect(t1).toHaveClass(/cd-selected/);
    await page.keyboard.press('Escape');
    await expect(t1).toBeHidden();
  });

  // spec: S47-02
  test('moving the selection closes a clean reply editor, so it never keeps a resolved card on screen', async ({ page }) => {
    const doc = seedDoc('web-r10', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id);
    await waitLive(page);
    const t1 = card(page, 't1');
    await mark(page, 't1').click();
    await t1.locator('[data-act="reply"]').click();
    await expect(t1.locator('.cd-editor-reply textarea')).toBeFocused();
    cli(h().agent, ['thread', 'resolve', doc.id, 't1']);
    await expect(t1).toHaveClass(/cd-resolved/);
    await mark(page, 't2').click();
    await expect(card(page, 't2')).toHaveClass(/cd-selected/);
    await expect(page.locator('.cd-editor')).toHaveCount(0);
    await expect(t1).toBeHidden();
  });

  // spec: S47-01, S47-05
  test('my own reply-and-resolve from the card keeps the card until I move on', async ({ page }) => {
    const doc = seedDoc('web-r11', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id);
    await waitLive(page);
    const t1 = card(page, 't1');
    await mark(page, 't1').click();
    await t1.locator('[data-act="reply"]').click();
    await t1.locator('.cd-editor-reply textarea').fill('Done and dusted.');
    await t1.locator('.cd-resolve-too').check();
    await page.keyboard.press('Enter');
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-cbody')).toContainText('Done and dusted.');
    await expect(t1).toHaveClass(/cd-resolved/);
    await expect(t1).toBeVisible();
    await expect(t1).toHaveClass(/cd-selected/);
    await expect(t1.locator('.cd-resline')).toContainText('resolved by Rowan Vale');
    await expect(page.locator('.cd-status')).toContainText('2 open');
    await expect(page.locator('.cd-res-n')).toHaveText('1');
    await page.keyboard.press('Escape');
    await expect(t1).toBeHidden();
  });

  // spec: S47-01, S47-03
  test('a live reopen of a kept Thread keeps it and clears the resolved mark', async ({ page }) => {
    const doc = seedDoc('web-r12', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id);
    await waitLive(page);
    const t1 = card(page, 't1');
    await mark(page, 't1').click();
    cli(h().agent, ['thread', 'resolve', doc.id, 't1']);
    await expect(t1).toHaveClass(/cd-resolved/);
    await expect(t1).toBeVisible();
    cli(h().agent, ['thread', 'reopen', doc.id, 't1']);
    await expect(t1).not.toHaveClass(/cd-resolved/);
    await expect(t1.locator('.cd-resline')).toHaveCount(0);
    await expect(t1).toBeVisible();
    await expect(t1).toHaveClass(/cd-selected/);
    await expect(page.locator('.cd-status')).toContainText('3 open');
    await expect(page.locator('.cd-res-n')).toHaveText('0');
    await page.keyboard.press('Escape');
    await expect(t1).toBeVisible();
  });

  // spec: S47-02
  test('a kept Thread deleted by another Member leaves the page as if it were never there', async ({ page }) => {
    const doc = seedDoc('web-r13', MD);
    await newThreads(doc.id, MD);
    await openDoc(page, doc.id);
    await waitLive(page);
    const t1 = card(page, 't1');
    await mark(page, 't1').click();
    cli(h().agent, ['thread', 'resolve', doc.id, 't1']);
    await expect(t1).toBeVisible();
    await expect(t1).toHaveClass(/cd-resolved/);
    cli(h().agent, ['thread', 'delete', doc.id, 't1']);
    await expect(t1).toHaveCount(0);
    await expect(page.locator('mark.cd-hl[data-t="t1"]')).toHaveCount(0);
    await expect(page.locator('.cd-thread.cd-selected')).toHaveCount(0);
    await expect(page.locator('.cd-status')).toContainText('2 open');
    await expect(page.locator('.cd-res-n')).toHaveText('0');
  });
});
