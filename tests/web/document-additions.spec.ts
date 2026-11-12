// The web's Document additions in the browser: Copy link (the Document's and a Thread's, the link
// `annotary link` prints, with the copy-by-hand fallback), Comment on the whole document, the
// Mentions page opening a Document at its Thread, and Delete (after a confirm) and Restore (on the
// deleted page and in one click from the list's Deleted filter). Every test pushes its own
// Document; the server markup per viewer is `crates/annotary/tests/web_document_additions.rs`.
import { test, expect, type Page } from '@playwright/test';
import { harness, cli, seedDoc, rangeOf } from './harness';
import { openDoc, waitLive } from './actions';

const h = () => harness();

const MD = `# Rollout notes

The cache warms before the cutover, so the first readers never wait.

We keep the old path for a week after the switch.
`;

async function linkOf(page: Page): Promise<string> {
  return page.evaluate(() => (window as unknown as { cd: { data: () => { link: string } } }).cd.data().link);
}

async function apiGet(page: Page, path: string): Promise<any> {
  return page.evaluate(async (p) => (await fetch(p, { credentials: 'same-origin' })).json(), path);
}

test.describe('copy link', () => {
  // spec: S48-01
  test('the More menu copies the Document link and a card copies its Thread link, both as annotary link prints them', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: h().url });
    const doc = seedDoc('web-add-link', MD);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(MD, 'cache warms'), '-m', 'Measured?']);
    const tid = 't1';
    await openDoc(page, doc.id);
    const link = await linkOf(page);
    expect(link).toBe(cli(h().owner, ['link', doc.id]).trim());
    await page.locator('#cd-docmenubtn').click();
    await expect(page.locator('#cd-docmenu')).toBeVisible();
    await page.locator('#cd-docmenu [data-doc-act="link"]').click();
    await expect(page.locator('#cd-docmenu')).toBeHidden();
    await expect(page.locator('#cd-toast')).toHaveText('Document link copied');
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(link);

    const card = page.locator(`.cd-thread[data-t="${tid}"]`);
    await card.locator('[data-act="link"]').click();
    await expect(page.locator('#cd-toast')).toHaveText('Thread link copied');
    const threadLink = await page.evaluate(() => navigator.clipboard.readText());
    expect(threadLink).toBe(`${link}#${tid}`);
    expect(threadLink).toBe(cli(h().owner, ['link', doc.id, tid]).trim());
  });

  // spec: S48-01
  test('where the clipboard is refused the link is shown selected to copy by hand', async ({ page }) => {
    const doc = seedDoc('web-add-link-by-hand', MD);
    await openDoc(page, doc.id);
    await page.evaluate(() => {
      Object.defineProperty(navigator, 'clipboard', { value: { writeText: () => Promise.reject(new Error('denied')) }, configurable: true });
      document.execCommand = () => false;
    });
    await page.locator('#cd-docmenubtn').click();
    await page.locator('#cd-docmenu [data-doc-act="link"]').click();
    const toast = page.locator('#cd-toast');
    await expect(toast).toContainText('Copy this link:');
    const input = toast.locator('input');
    await expect(input).toHaveValue(await linkOf(page));
    await expect(input).toBeFocused();
    expect(await input.evaluate((el: HTMLInputElement) => [el.selectionStart, el.selectionEnd, el.value.length])).toEqual([0, (await linkOf(page)).length, (await linkOf(page)).length]);
    // A click in the field keeps the notice open.
    await input.click();
    await expect(toast).toBeVisible();
  });
});

test.describe('comment on the whole document', () => {
  // spec: S48-02
  test('the menu opens a composer in the Whole document strip; posting makes a Document-level Thread', async ({ page }) => {
    const doc = seedDoc('web-add-doc-comment', MD);
    await openDoc(page, doc.id);
    await waitLive(page);
    await page.locator('#cd-docmenubtn').click();
    await page.locator('#cd-docmenu [data-doc-act="comment"]').click();
    const composer = page.locator('.cd-strip-doc .cd-composer');
    await expect(composer).toBeVisible();
    await expect(composer.locator('.cd-quote')).toHaveText('whole document');
    const ta = composer.locator('textarea');
    await expect(ta).toBeFocused();
    // The strip sits under the title, in the margin (layout A).
    const strip = page.locator('.cd-strip-doc');
    expect(await strip.evaluate((e) => getComputedStyle(e).position)).toBe('absolute');
    await ta.fill('This needs a rollback section.');
    await page.keyboard.press('Control+Enter');
    const card = page.locator('.cd-strip-doc .cd-thread[data-t="t1"]');
    await expect(card).toBeVisible();
    await expect(card).not.toHaveClass(/cd-pending/);
    await expect(card.locator('.cd-quote')).toHaveText('whole document');
    await expect(card.locator('.cd-cbody')).toContainText('This needs a rollback section.');
    await expect(page.locator('.cd-composer')).toHaveCount(0);
    const j = await apiGet(page, `/api/v1/docs/${doc.id}/threads`);
    expect(j.threads.map((t: any) => [t.id, t.subject])).toEqual([['t1', 'document']]);
  });

  // spec: S48-02
  test('a refused post keeps the text with the error', async ({ page }) => {
    const doc = seedDoc('web-add-doc-comment-fail', MD);
    await openDoc(page, doc.id);
    await page.route('**/api/v1/docs/*/threads', (route) =>
      route.request().method() === 'POST'
        ? route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'internal', message: 'the disk is full' }) })
        : route.continue());
    await page.locator('#cd-docmenubtn').click();
    await page.locator('#cd-docmenu [data-doc-act="comment"]').click();
    await page.locator('.cd-composer textarea').fill('Keep me.');
    await page.keyboard.press('Control+Enter');
    const again = page.locator('.cd-strip-doc .cd-composer');
    await expect(again.locator('.cd-err')).toHaveText('Comment not posted: the disk is full');
    await expect(again.locator('textarea')).toHaveValue('Keep me.');
  });
});

test.describe('mentions', () => {
  // spec: S48-03
  test('the Mentions page lists the Thread and opens the Document at it', async ({ page }) => {
    const doc = seedDoc('web-add-mentions', MD);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(MD, 'old path'), '-m', 'How long, @rowan?']);
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto(`${h().url}/`);
    await page.locator('#cd-nav-mentions').click();
    await expect(page).toHaveURL(/\/mentions$/);
    const row = page.locator(`.cd-mrow[data-doc="${doc.id}"]`);
    await expect(row).toHaveCount(1);
    await expect(row.locator('.cd-quote q')).toHaveText('old path');
    await expect(row.locator('.cd-msaid')).toBeVisible();
    await expect(row.locator('.cd-msaid')).toContainText('How long, @rowan?');
    // Newest first: the Thread just made heads the list.
    await expect(page.locator('.cd-mrow').first()).toHaveAttribute('data-doc', doc.id);
    await row.locator('a').click();
    await expect(page).toHaveURL(new RegExp(`/d/${doc.id}#t1$`));
    await page.waitForSelector('.cd-app.cd-ready');
    await expect(page.locator('.cd-thread[data-t="t1"]')).toHaveClass(/cd-selected/);
  });
});

test.describe('delete and restore', () => {
  // spec: S48-04
  test('Delete asks first, then the page is deleted and read-only; Restore brings it back', async ({ page }) => {
    const doc = seedDoc('web-add-delete', MD);
    await openDoc(page, doc.id);
    await page.locator('#cd-docmenubtn').click();
    await page.locator('#cd-docmenu [data-doc-act="delete"]').click();
    const confirm = page.locator('#cd-delconfirm');
    await expect(confirm).toBeVisible();
    await expect(confirm).toContainText('Delete this Document?');
    await confirm.locator('[data-doc-act="delete-no"]').click();
    await expect(page.locator('#cd-docmenu')).toBeHidden();
    expect((await apiGet(page, `/api/v1/docs/${doc.id}`)).deleted_at).toBeNull();

    await page.locator('#cd-docmenubtn').click();
    await expect(confirm).toBeHidden();
    await page.locator('#cd-docmenu [data-doc-act="delete"]').click();
    await confirm.locator('[data-doc-act="delete-yes"]').click();
    await expect(page.locator('#cd-restore')).toBeVisible();
    await expect(page.locator('.cd-hdr .cd-chip.cd-warn')).toHaveText('deleted');
    await expect(page.locator('#cd-docmenu [data-doc-act="comment"]')).toHaveCount(0);
    expect((await apiGet(page, `/api/v1/docs/${doc.id}`)).deleted_at).not.toBeNull();

    await page.locator('#cd-restore').click();
    await expect(page.locator('#cd-restore')).toHaveCount(0);
    await expect(page.locator('#cd-docmenu [data-doc-act="delete"]')).toHaveCount(1);
    expect((await apiGet(page, `/api/v1/docs/${doc.id}`)).deleted_at).toBeNull();
  });

  // spec: S48-04
  test('the Deleted filter restores in one click', async ({ page }) => {
    const doc = seedDoc('web-add-restore-row', MD);
    cli(h().owner, ['delete', doc.id]);
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto(`${h().url}/?name=web-add-restore-row&deleted=1`);
    await page.waitForSelector('.cd-app.cd-ready');
    const row = page.locator(`tr.cd-docrow[data-id="${doc.id}"]`);
    await row.locator('button.cd-restore').click();
    await expect(row.locator('.cd-restored')).toHaveText('restored');
    await expect(row.locator('.cd-deleted')).toHaveCount(0);
    expect((await apiGet(page, `/api/v1/docs/${doc.id}`)).deleted_at).toBeNull();
  });
});
