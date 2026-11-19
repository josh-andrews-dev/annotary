// The review handoff on the real data (S49): an agent Member added to the copy saves
// `review_handoff=hook` for its own install (`--device this`), registers a `--once` hook to a
// loopback receiver, and the owner's Send back in the real Document page reaches the receiver
// once, signed, with no Comment text; then a background `wait` (Claude Code's recipe) exits on
// the owner's Approve. The copy may call only loopback (verify_live.py sets ONLY_HOSTS), so the
// live hooks in it never wake a real agent.
import * as crypto from 'node:crypto';
import * as fs from 'node:fs';
import * as http from 'node:http';
import * as path from 'node:path';
import { execFileSync, spawn } from 'node:child_process';
import { test } from '@playwright/test';
import { URL, cli, cliJson, expect, openDoc, shot } from '../lib';

type Got = { headers: http.IncomingHttpHeaders; body: Buffer };

test('Send back wakes a --once hook signed and body-free; Approve ends a background wait', async ({ page }) => {
  // A receiver on loopback that records every request.
  const got: Got[] = [];
  const server = http.createServer((req, res) => {
    const parts: Buffer[] = [];
    req.on('data', (c) => parts.push(c));
    req.on('end', () => {
      got.push({ headers: req.headers, body: Buffer.concat(parts) });
      res.writeHead(200).end();
    });
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  const port = (server.address() as { port: number }).port;

  // The agent Member and its own CLI home (a throwaway profile on the copy).
  const owner = cliJson<{ token: string }>(['token', 'mint', '--name', 'verify-hooks']).token;
  const handle = `verify-hook-${Date.now().toString(36)}`;
  const added = await fetch(URL + '/api/v1/members', {
    method: 'POST',
    headers: { Authorization: 'Bearer ' + owner, 'Content-Type': 'application/json' },
    body: JSON.stringify({ handle, name: 'Verify Hook Agent' }),
  });
  expect(added.status).toBe(201);
  const agentToken = ((await added.json()) as { token: string }).token;
  const dir = fs.mkdtempSync(path.join(process.env.VERIFY_TMP!, 'hooks-'));
  const home = path.join(dir, 'home');
  fs.mkdirSync(home, { mode: 0o700 });
  fs.writeFileSync(path.join(home, 'config.toml'), `[profiles.agent]\nserver = "${URL}"\ntoken = "${agentToken}"\n`, { mode: 0o600 });
  const env = { PATH: process.env.PATH ?? '', HOME: dir, ANNOTARY_HOME: home };
  const agent = (args: string[], input?: string) =>
    execFileSync(process.env.VERIFY_BIN!, ['-p', 'agent', ...args], { env, encoding: 'utf8', input });

  // The setup choice, saved for this install.
  expect(agent(['settings', 'get', 'review_handoff', '--device', 'this'])).toMatch(/^review_handoff\task\tdefault\t/);
  agent(['settings', 'set', 'review_handoff', 'hook', '--device', 'this']);
  agent(['settings', 'set', 'review_wake', 'background-wait', '--device', 'this']);
  expect(agent(['settings', 'get', 'review_handoff', '--device', 'this'])).toMatch(/^review_handoff\thook\tdevice\t/);
  expect(fs.readFileSync(path.join(home, 'config.toml'), 'utf8')).toMatch(/^device = "/);

  // The agent hands over a Document; the owner comments on it.
  const file = path.join(dir, 'verify-live-handoff.md');
  fs.writeFileSync(file, '# Verify live handoff\n\nAn agent handed this over for review.\n');
  const vault = cliJson<{ name: string }[]>(['vault', 'list'])[0].name;
  agent(['push', file, '--vault', vault, '-m', 'for review']);
  const id = JSON.parse(agent(['status', file, '--json'])).id as string;
  const secret = crypto.randomBytes(24).toString('hex');
  const out = agent(['hook', 'add', id, '--once', '--on', 'action,mention'], `http://127.0.0.1:${port}/hook\n${secret}\n`);
  expect(out.split('\t')[1].trim()).toBe(secret);
  cli(['thread', 'new', id, '--document', '-m', 'VERIFYBODYTEXT say when the cache is cleared']);

  // The owner presses Send back in the page.
  await openDoc(page, id);
  await page.locator('#cd-actbtns .cd-actbtn', { hasText: 'Send back' }).click();
  await expect(page.locator('#cd-actions .cd-chip.cd-action')).toContainText('Send back');
  await expect.poll(() => got.length, { timeout: 15_000 }).toBe(1);
  const g = got[0];
  const sig = 'sha256=' + crypto.createHmac('sha256', secret).update(g.body).digest('hex');
  expect(g.headers['x-annotary-signature']).toBe(sig);
  const body = JSON.parse(g.body.toString('utf8'));
  expect(body).toMatchObject({ v: 1, kind: 'action', action: 'send-back', doc: id, thread: null });
  expect(g.body.toString('utf8')).not.toContain('VERIFYBODYTEXT');
  // `--once`: gone after the delivery.
  await expect.poll(() => agent(['hook', 'list']).trim(), { timeout: 10_000 }).toBe('');
  await shot(page, 'review-handoff-sent-back');

  // Claude Code's recipe: a background `wait`, ended by the owner's Approve.
  const waiter = spawn(process.env.VERIFY_BIN!, ['-p', 'agent', 'wait', id, '--on', 'action,mention', '--timeout', '2m'], { env });
  let line = '';
  waiter.stdout.on('data', (c) => (line += c.toString()));
  const exited = new Promise<number | null>((r) => waiter.on('exit', r));
  await new Promise((r) => setTimeout(r, 1500));
  await page.reload();
  await page.waitForSelector('.cd-app.cd-ready');
  await page.locator('#cd-actbtns .cd-actbtn', { hasText: 'Approve' }).click();
  expect(await exited).toBe(0);
  expect(line).toMatch(new RegExp(`\\taction\\t[^\\t]*\\tapprove v1 by @`));
  expect(got.length).toBe(1);
  server.close();
});
