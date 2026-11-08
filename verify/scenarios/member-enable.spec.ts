// `member enable` on the real data (the Hermes plugin brings back a returning profile's Member): a
// Member added to the copy and disabled shows Enable on the real Members screen; Enable brings the
// same Member back, active, its old Token still refused; the CLI's enable of an active Member is a
// note, not an error.
import { test } from '@playwright/test';
import { URL, cli, cliJson, expect, shot } from '../lib';

test('a disabled Member comes back with Enable, its old Token stays refused; enabling an active one is a note', async ({ page }) => {
  const owner = cliJson<{ token: string }>(['token', 'mint', '--name', 'verify-enable']).token;
  const call = async (method: string, route: string, token: string, body?: unknown) => {
    const r = await fetch(URL + route, {
      method,
      headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    return { status: r.status, json: await r.json().catch(() => null) };
  };
  const handle = `verify-back-${Date.now().toString(36)}`;
  const added = await call('POST', '/api/v1/members', owner, { handle, name: 'Verify Comes Back' });
  expect(added.status).toBe(201);
  const old = added.json.token as string;
  expect((await call('POST', `/api/v1/members/${handle}/disable`, owner, {})).status).toBe(200);

  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto('/settings/server/members');
  const me = page.locator(`.cd-mem[data-handle="${handle}"]`);
  await expect(me).toHaveClass(/cd-mem-disabled/);
  await me.locator('[data-act=enable]').click();
  await me.locator('[data-slot=rename] [data-inline=yes]').click();
  await expect(me).not.toHaveClass(/cd-mem-disabled/);
  await expect(me.locator('[data-act=disable]')).toHaveCount(1);
  await me.screenshot({ path: `${process.env.VERIFY_OUT}/member-enabled-card.png` });
  await shot(page, 'members-after-enable');

  expect((await call('GET', '/api/v1/me', old)).status).toBe(401);
  const note = cli(['member', 'enable', handle]);
  expect(note).toContain(handle);
});
