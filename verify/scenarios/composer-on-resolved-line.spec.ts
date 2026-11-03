// S36-01 on the real data: selecting text on a line whose only Threads are resolved and pressing
// `c` opens a composer you can see and type in; posting shows the card, and the resolved Thread
// stays behind the toggle. Uses such a line already in the copy, or makes one.
import * as fs from 'node:fs';
import * as path from 'node:path';
import { test, type Page } from '@playwright/test';
import { cli, documents, expect, openDoc, shot } from '../lib';

/** The id of a resolved span Thread alone on its line, whose mark holds selectable text. */
async function hiddenLine(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    for (const m of document.querySelectorAll('.cd-strip.cd-all-resolved .cd-thread')) {
      const tid = m.getAttribute('data-t');
      const mark = document.querySelector(`mark.cd-hl.cd-res[data-t="${tid}"]`);
      if (mark && (mark.textContent || '').trim().length > 1 && !mark.closest('.cd-strip')) return tid;
    }
    return null;
  });
}

test('a composer on a line whose only Threads are resolved shows and posts', async ({ page }) => {
  let found: { id: string; tid: string } | null = null;
  for (const d of documents()) {
    await openDoc(page, d.id);
    const tid = await hiddenLine(page);
    if (tid) { found = { id: d.id, tid }; break; }
  }
  test.info().annotations.push({ type: 'data', description: found ? `existing resolved Thread ${found.tid} on ${found.id}` : 'made one' });
  if (!found) {
    const d = documents()[0];
    const dir = fs.mkdtempSync(path.join(process.env.VERIFY_TMP!, 'pull-'));
    cli(['pull', d.id, '--out', dir]);
    const md = fs.readdirSync(dir).find((f) => f.endsWith('.md'))!;
    const lines = fs.readFileSync(path.join(dir, md), 'utf8').split('\n');
    const n = lines.findIndex((l) => /^[A-Za-z]{6}/.test(l));
    const tid = JSON.parse(cli(['thread', 'new', d.id, `${n + 1}:1-${n + 1}:6`, '-m', 'verify-live S36-01', '--json', 'id'])).id;
    cli(['thread', 'resolve', d.id, tid]);
    found = { id: d.id, tid };
    await openDoc(page, d.id);
  }
  const toggle = page.locator('#cd-resolved');
  if (await toggle.isChecked()) await toggle.uncheck();
  const resolved = page.locator(`.cd-thread[data-t="${found.tid}"]`);
  await expect(resolved).toBeHidden();

  await page.evaluate((tid) => {
    const mark = document.querySelector(`mark.cd-hl.cd-res[data-t="${tid}"]`)!;
    const range = document.createRange();
    range.selectNodeContents(mark);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
  }, found.tid);
  await expect(page.locator('#cd-pill')).toBeVisible();
  await page.keyboard.press('c');
  const box = page.locator('.cd-composer textarea');
  await expect(box).toBeVisible();
  await expect(box).toBeFocused();
  await box.fill('verify-live S36-01');
  await box.scrollIntoViewIfNeeded();
  await shot(page, 'composer-on-resolved-line');

  await page.keyboard.press('Control+Enter');
  const card = page.locator('.cd-thread:not(.cd-composer):not(.cd-pending)', { hasText: 'verify-live S36-01' }).first();
  await expect(card).toBeVisible();
  await expect(resolved).toBeHidden();
  await shot(page, 'composer-on-resolved-line-posted');
});
