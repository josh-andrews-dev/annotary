// Questions on the real data: a new Document with a decisions table is pushed into one of the
// copy's Vaults, answered with a click in the browser and through the CLI, read with `show`, pulled
// with the answers written into its Decided cells, refused when pushed without them, and pushed
// filled, which makes its Questions decided.
import { test } from '@playwright/test';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { OUT, cli, cliJson, documents, expect, openDoc, shot } from '../lib';

const MD = `# Verify: decisions

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

test('a decisions table is answered, shown, pulled filled and pushed', async ({ page }) => {
  const vault = (JSON.parse(cli(['list', '--json', 'id,vault']) || '[]')[0] ?? { vault: undefined }).vault as string | undefined;
  expect(documents().length).toBeGreaterThan(0);
  const dir = fs.mkdtempSync(path.join(process.env.VERIFY_TMP!, 'questions-'));
  const file = path.join(dir, 'verify-decisions.md');
  fs.writeFileSync(file, MD);
  const pushed = cliJson<{ id: string; version: number }>(['push', file, ...(vault ? ['--vault', vault] : [])]);
  const id = pushed.id;
  expect(cli(['show', id])).toContain('questions: D1=- D2=-');

  // A click in the browser answers D1; the CLI answers D2.
  await openDoc(page, id);
  await page.locator('.cd-qrow[data-q="D1"] .cd-qopt[data-o="B"]').click();
  await expect(page.locator('.cd-qrow[data-q="D1"] .cd-qwho')).toHaveText(/^B @/);
  cli(['answer', id, 'D2', 'C']);
  await expect.poll(() => cli(['show', id])).toContain('questions: D1=B D2=C');
  await page.reload();
  await page.waitForSelector('.cd-app.cd-ready');
  await expect(page.locator('#cd-qsum')).toHaveText('2 of 2 answered');
  await shot(page, 'questions-document');
  await page.emulateMedia({ colorScheme: 'dark' });
  await openDoc(page, id, 390);
  await page.locator('.cd-qrow[data-q="D1"]').scrollIntoViewIfNeeded();
  await shot(page, 'questions-phone-dark');
  await page.emulateMedia({ colorScheme: 'light' });

  // An edited file whose Decided cells are still empty is refused with the rows to put in.
  fs.writeFileSync(file, MD.replace('A new VM.', 'A new VM, bigger.'));
  let refused = '';
  try { cli(['push', file]); } catch (e: any) { refused = String(e.stderr ?? ''); }
  expect(refused).toContain('are not in the Decided cells');
  // `pull` writes them in; the filled file pushes, and the Questions are decided.
  cli(['pull', id, '--out', dir, '--force']);
  const filled = fs.readFileSync(file, 'utf8');
  expect(filled).toMatch(/\| D1 \| A, B \| A \| B \(@[a-z0-9-]+, \d{4}-\d\d-\d\d\) \|/);
  expect(filled.split('\n').length).toBe(MD.split('\n').length);
  expect(cli(['push', file, '-m', 'record the decisions'])).toContain('\tv2');
  expect(cli(['show', id])).toContain('questions: D1=decided D2=decided');
  await openDoc(page, id);
  await expect(page.locator('.cd-qrow[data-q="D1"]')).toHaveClass(/cd-qdone/);
  await shot(page, 'questions-decided');
  fs.writeFileSync(path.join(OUT, 'questions-filled.md'), filled);
});
