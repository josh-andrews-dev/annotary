// The Compare diff over the API on the real data (`GET /api/v1/docs/{id}/diff`, what the iPhone and
// iPad app reads with a Bearer Token). For every real Document with more than one Version, head−1 →
// head and v1 → head: the API resolves the page's pairs, its fragment is the page's diff with only
// the page's own additions (its notes, Asset links opening in a new tab), its numbers are the page's
// and its change points run 1…changes. A Member without the Vault's read right gets what reading a
// Version gets; an unknown Version is 404 `not_found`.
import { test } from '@playwright/test';
import { URL, cliJson, documents, expect } from '../lib';

interface V { number: number }
interface Summary { lines_added: number; lines_removed: number; changes: number; too_large: boolean }
interface Diff { id: string; from: number; to: number; head: number; html: string | null; summary: Summary | null }

test('the API answers the diff page\'s pair, fragment and numbers for every real Document', async () => {
  test.setTimeout(600_000);
  const owner = cliJson<{ token: string }>(['token', 'mint', '--name', 'verify-diff-api']).token;
  const get = (route: string, token: string, accept = 'application/json') =>
    fetch(URL + route, { headers: { Authorization: 'Bearer ' + token, Accept: accept } });
  const multi = documents()
    .map((d) => ({ ...d, versions: cliJson<V[]>(['versions', d.id]) ?? [] }))
    .filter((d) => d.versions.length > 1);
  expect(multi.length, 'the live data has a Document with more than one Version').toBeGreaterThan(0);
  let pairs = 0;
  for (const d of multi) {
    const head = Math.max(...d.versions.map((v) => v.number));
    const r = await get(`/api/v1/docs/${d.id}/diff`, owner);
    expect(r.status, d.id).toBe(200);
    const def = (await r.json()) as Diff;
    expect([def.from, def.to, def.head], `${d.id}: the default pair`).toEqual([head - 1, head, head]);
    for (const from of [...new Set([head - 1, 1])]) {
      const q = `from=${from}&to=${head}`;
      const j = (await (await get(`/api/v1/docs/${d.id}/diff?${q}`, owner)).json()) as Diff;
      expect([j.from, j.to], `${d.id} ${q}`).toEqual([from, head]);
      expect(j.summary, `${d.id} ${q}: compared`).not.toBeNull();
      expect(j.summary!.too_large, `${d.id} ${q}: not too large`).toBe(false);
      const page = await (await get(`/d/${d.id}/diff?${q}`, owner, 'text/html')).text();
      const open = '<article class="cd-doc cd-dxdoc" id="cd-dxdoc">';
      const at = page.indexOf(open);
      expect(at, `${d.id} ${q}: the page's diff`).toBeGreaterThan(0);
      let part = page.slice(at + open.length, page.indexOf('</article></main>', at));
      const html = j.html!.replaceAll('<a href="/a/', '<a target="_blank" rel="noopener" href="/a/');
      expect(part.endsWith(html), `${d.id} ${q}: the page's diff is the API's`).toBe(true);
      part = part.slice(0, part.length - html.length).replace(/<p class="cd-dx-note"[^>]*>.*?<\/p>/g, '');
      expect(part, `${d.id} ${q}: only notes ahead of it`).toBe('');
      const num = (re: RegExp) => Number(page.match(re)?.[1]);
      expect(
        [num(/<b class="cd-dxplus">\+(\d+)</), num(/<b class="cd-dxminus">−(\d+)</), num(/data-changes="(\d+)"/)],
        `${d.id} ${q}: the numbers`,
      ).toEqual([j.summary!.lines_added, j.summary!.lines_removed, j.summary!.changes]);
      const points = [...j.html!.matchAll(/data-cd-dx="(\d+)"/g)].map((m) => Number(m[1]));
      expect(points, `${d.id} ${q}: change points`).toEqual(Array.from({ length: j.summary!.changes }, (_, i) => i + 1));
      pairs++;
    }
  }
  test.info().annotations.push({ type: 'data', description: `${multi.length} Documents, ${pairs} pairs` });

  // The same errors as reading a Version.
  const d = multi[0];
  const nf = await get(`/api/v1/docs/${d.id}/diff?from=999999`, owner);
  expect([nf.status, ((await nf.json()) as { error: string }).error]).toEqual([404, 'not_found']);
  const send = (method: string, route: string, body: unknown) =>
    fetch(URL + route, {
      method,
      headers: { Authorization: 'Bearer ' + owner, 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
  // A Member whose only Grant is on a new, empty Vault.
  const tag = Date.now().toString(36);
  expect((await send('POST', '/api/v1/vaults', { name: `verify-diff-${tag}` })).status).toBe(201);
  const added = await send('POST', '/api/v1/members', { handle: `verify-diff-${tag}`, name: 'Verify Diff Reader' });
  expect(added.status).toBe(201);
  const stranger = ((await added.json()) as { token: string }).token;
  const grants = await send('PUT', `/api/v1/members/verify-diff-${tag}/grants`, {
    grants: [{ vault: `verify-diff-${tag}`, rights: ['read'] }],
  });
  expect(grants.status).toBe(200);
  const version = await get(`/api/v1/docs/${d.id}/versions/1`, stranger);
  const diff = await get(`/api/v1/docs/${d.id}/diff`, stranger);
  expect(version.status, 'a Member without the right cannot read the Version').not.toBe(200);
  expect([diff.status, ((await diff.json()) as { error: string }).error]).toEqual([
    version.status,
    ((await version.json()) as { error: string }).error,
  ]);
});
