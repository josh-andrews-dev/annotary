// The web's Document additions on the live data: Copy link gives exactly what `annotary link`
// prints for a real Document and one of its Threads, the More menu comments on the whole
// Document, the Mentions page lists a Thread another Member addressed to the owner and opens the
// Document at it, and Delete (after its confirm) and Restore (on the page, then from the list's
// Deleted filter) round-trip a real Document.
//
// What it creates: one Token, one Member, two Document-level Threads on a real Document (both
// tombstoned at the end), and a delete and two restores of that Document, which ends live. All
// of it lives only on the copy `make verify-live` deletes.
import { test } from '@playwright/test';
import { URL, cli, cliJson, documents, expect, openDoc, shot } from '../lib';

test('Copy link, a whole-Document comment, Mentions, and Delete and Restore on a real Document', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: URL });
  const owner = cliJson<{ token: string }>(['token', 'mint', '--name', 'verify-document-additions']).token;
  const call = async (method: string, route: string, token: string, body?: unknown) => {
    const r = await fetch(URL + route, {
      method,
      headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    return { status: r.status, json: await r.clone().json().catch(() => null) };
  };
  const handle = `verify-additions-${Date.now().toString(36)}`;
  const added = await call('POST', '/api/v1/members', owner, { handle, name: 'Verify Additions' });
  expect(added.status).toBe(201);
  const token = added.json.token as string;

  // A real Document that already has an open Thread, so a card's Copy link is tried too.
  const docs = documents();
  expect(docs.length, 'the copy has Documents').toBeGreaterThan(0);
  const withThread = docs.find((d) => (cliJson<{ id: string }[]>(['thread', 'list', d.id]) ?? []).length > 0);
  const doc = withThread ?? docs[0];
  test.info().annotations.push({ type: 'data', description: `Document ${doc.id}` });

  // ---- Copy link: the More menu and a card, each what `annotary link` prints ----
  await openDoc(page, doc.id);
  await page.waitForSelector('.cd-app.cd-live');
  const link = cli(['link', doc.id]).trim();
  await page.locator('#cd-docmenubtn').click();
  await expect(page.locator('#cd-docmenu [data-doc-act="link"]')).toBeVisible();
  await shot(page, 'additions-more-menu');
  await page.locator('#cd-docmenu [data-doc-act="link"]').click();
  await expect(page.locator('#cd-toast')).toHaveText('Document link copied');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(link);
  const firstCard = page.locator('.cd-thread:not(.cd-resolved)').first();
  if (withThread) {
    const tid = (await firstCard.getAttribute('data-t'))!;
    await firstCard.locator('[data-act="link"]').click();
    await expect(page.locator('#cd-toast')).toHaveText('Thread link copied');
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(cli(['link', doc.id, tid]).trim());
  }

  // ---- Comment on the whole Document ----
  await page.locator('#cd-docmenubtn').click();
  await page.locator('#cd-docmenu [data-doc-act="comment"]').click();
  const composer = page.locator('.cd-strip-doc .cd-composer');
  await expect(composer).toBeVisible();
  await composer.locator('textarea').fill('A whole-Document comment from the verify run.');
  await shot(page, 'additions-document-composer');
  await page.keyboard.press('Control+Enter');
  const posted = page.locator('.cd-strip-doc .cd-thread:not(.cd-pending):not(.cd-composer)', { hasText: 'A whole-Document comment from the verify run.' });
  await expect(posted).toBeVisible();
  const ownTid = (await posted.getAttribute('data-t'))!;
  expect((await call('GET', `/api/v1/docs/${doc.id}/threads/${ownTid}`, owner)).json.subject).toBe('document');

  // ---- Mentions: another Member addresses the owner; the page lists it first and opens it ----
  const me = await page.evaluate(() => (window as any).cd.data().me as string);
  const made = await call('POST', `/api/v1/docs/${doc.id}/threads`, token, { subject: 'document', body: `@${me} a mention from the verify run.` });
  expect(made.status).toBe(201);
  const mentionTid = made.json.id as string;
  await page.goto('/mentions');
  await page.waitForSelector('.cd-app.cd-ready');
  const first = page.locator('.cd-mrow').first();
  await expect(first).toHaveAttribute('data-doc', doc.id);
  await expect(first).toHaveAttribute('data-t', mentionTid);
  const api = (await call('GET', '/api/v1/mentions', owner)).json.mentions as { thread: { id: string }; doc: { id: string } }[];
  await expect(page.locator('.cd-mrow')).toHaveCount(api.length);
  await shot(page, 'additions-mentions');
  await first.locator('a').click();
  await page.waitForSelector('.cd-app.cd-ready');
  await expect(page.locator(`.cd-thread[data-t="${mentionTid}"]`)).toHaveClass(/cd-selected/);

  // ---- Delete, after its confirm; Restore on the deleted page ----
  await page.locator('#cd-docmenubtn').click();
  await page.locator('#cd-docmenu [data-doc-act="delete"]').click();
  await expect(page.locator('#cd-delconfirm')).toBeVisible();
  await shot(page, 'additions-delete-confirm');
  await page.locator('#cd-delconfirm [data-doc-act="delete-yes"]').click();
  await expect(page.locator('#cd-restore')).toBeVisible();
  expect((await call('GET', `/api/v1/docs/${doc.id}`, owner)).json.deleted_at).not.toBeNull();
  await shot(page, 'additions-deleted-restore');
  await page.locator('#cd-restore').click();
  await expect(page.locator('#cd-restore')).toHaveCount(0);
  expect((await call('GET', `/api/v1/docs/${doc.id}`, owner)).json.deleted_at).toBeNull();

  // ---- Restore in one click from the list's Deleted filter ----
  cli(['delete', doc.id]);
  await page.goto('/?deleted=1');
  await page.waitForSelector('.cd-app.cd-ready');
  const row = page.locator(`tr.cd-docrow[data-id="${doc.id}"]`);
  await row.locator('button.cd-restore').click();
  await expect(row.locator('.cd-restored')).toHaveText('restored');
  await shot(page, 'additions-list-restored');
  expect((await call('GET', `/api/v1/docs/${doc.id}`, owner)).json.deleted_at).toBeNull();

  // Cleanup: tombstone both Threads, so they show nowhere on the copy.
  cli(['thread', 'delete', doc.id, ownTid]);
  cli(['thread', 'delete', doc.id, mentionTid]);
});
