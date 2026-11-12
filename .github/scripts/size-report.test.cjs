"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const sr = require("./size-report.cjs");

function report(values = {}, ceilings = {}) {
  const base = {
    "view.bytes": ["Document view: bytes", "bytes", 1000, 1100],
    "view.elements": ["Document view: elements", "count", 50, null],
    "reply.event.bytes": ["Reply: live event bytes", "bytes", 300, 330],
  };
  return {
    schema: 1,
    fixture: { scale: "full", seed: 1, lines: 5000, versions: 5, threads: 500, replies: 600, list_documents: 60 },
    metrics: Object.entries(base).map(([id, [label, unit, value, ceiling]]) => ({
      id,
      label,
      unit,
      value: id in values ? values[id] : value,
      ceiling: id in ceilings ? ceilings[id] : ceiling,
    })),
  };
}

test("the committed report is valid", { skip: process.env.SIZE_REPORT_TEST_NO_FILES === "1" }, () => {
  const text = fs.readFileSync(path.join(__dirname, "../../perf/size-report.json"), "utf8");
  const r = sr.parseReport(text);
  assert.ok(r.metrics.some((m) => m.ceiling !== null));
});

test("a report is validated strictly as data", () => {
  assert.equal(sr.validateReport(report()).schema, 1);
  const bad = [
    { ...report(), extra: "@everyone" },
    { ...report(), schema: 2 },
    { ...report(), fixture: { ...report().fixture, scale: "<b>" } },
    { ...report(), fixture: { ...report().fixture, note: "x" } },
    { ...report(), metrics: [] },
  ];
  const withMetric = (patch) => {
    const r = report();
    r.metrics[0] = { ...r.metrics[0], ...patch };
    return r;
  };
  bad.push(
    withMetric({ label: "bytes | injected | row" }),
    withMetric({ label: "<img src=x onerror=alert(1)>" }),
    withMetric({ label: "ping @someone" }),
    withMetric({ label: "a\nb" }),
    withMetric({ label: "[link](https://example.com)" }),
    withMetric({ id: "View.Bytes" }),
    withMetric({ id: "view.elements" }),
    withMetric({ unit: "ms" }),
    withMetric({ value: -1 }),
    withMetric({ value: 1.5 }),
    withMetric({ value: "1000" }),
    withMetric({ ceiling: 1e13 }),
    withMetric({ note: 1 }),
  );
  for (const r of bad) assert.throws(() => sr.validateReport(r), /size-report\.json/, JSON.stringify(r).slice(0, 120));
  assert.throws(() => sr.parseReport("{"), /not JSON/);
  assert.throws(() => sr.parseReport(" ".repeat(65 * 1024)), /64 KiB/);
});

test("the Size growth line is read from the prose only", () => {
  assert.equal(sr.growthReason("## Summary\n\nSize growth: the avatars add a picture per card"), "the avatars add a picture per card");
  assert.equal(sr.growthReason("- **Size growth:** one more button"), "one more button");
  assert.equal(sr.growthReason("**Size growth**: one more button"), "one more button");
  assert.equal(sr.growthReason("<!-- Size growth: a hint in the template -->"), null);
  assert.equal(sr.growthReason("```\nSize growth: inside code\n```"), null);
  assert.equal(sr.growthReason("Size growth:"), null);
  assert.equal(sr.growthReason("Size growth: -"), null);
  assert.equal(sr.growthReason(null), null);
});

test("growth fails without a reason and passes with one; a ceiling always fails", () => {
  const base = report();
  assert.equal(sr.verdict({ base, head: report(), body: "" }).ok, true);
  assert.equal(sr.verdict({ base, head: report({ "view.bytes": 900 }), body: "" }).ok, true, "shrinking is fine");
  const grew = report({ "view.elements": 51 });
  assert.equal(sr.verdict({ base, head: grew, body: "" }).ok, false, "a number without a ceiling grew");
  assert.equal(sr.verdict({ base, head: grew, body: "Size growth: one more element per card" }).ok, true);
  const over = report({ "view.bytes": 1101 });
  assert.equal(sr.verdict({ base, head: over, body: "Size growth: on purpose" }).ok, false);
  assert.equal(sr.verdict({ base: null, head: over, body: "" }).ok, false, "no base: the ceilings still apply");
  assert.equal(sr.verdict({ base: null, head: report(), body: "" }).ok, true, "the first report");
  const raised = report({}, { "view.bytes": 1200 });
  assert.equal(sr.verdict({ base, head: raised, body: "" }).ok, false, "a raised ceiling counts as growth");
  assert.equal(sr.verdict({ base, head: raised, body: "Size growth: raised by the rule" }).ok, true);
  assert.equal(sr.verdict({ base, head: report({}, { "view.elements": 60 }), body: "" }).ok, true, "a new ceiling is a tightening");
  assert.equal(sr.verdict({ base, head: report({}, { "view.bytes": 1050 }), body: "" }).ok, true, "a lowered ceiling");
  const dropped = report({}, { "view.bytes": null });
  const droppedMd = sr.renderComment({ base, head: dropped, baseSha: "a".repeat(40), headSha: "b".repeat(40), body: "" });
  const [mainTable, collapsed = ""] = droppedMd.split("<details>");
  assert.match(mainTable, /\| Document view: bytes \| 1,000 \| 1,000 \| ±0 \(0\.0%\) \| — \| ⚠️ ceiling \|/, "a removed ceiling is in the main table");
  assert.doesNotMatch(collapsed, /Document view: bytes/, "and not in the collapsed one");
  assert.match(droppedMd, /1 number grew or had its guard weakened with no `Size growth:` line/);
  const withReason = sr.renderComment({ base, head: dropped, baseSha: "a".repeat(40), headSha: "b".repeat(40), body: "Size growth: moved" });
  assert.match(withReason, /⚠️ 1 number grew or had its guard weakened; the body's `Size growth:` line gives the reason\./);
  assert.equal(sr.verdict({ base, head: dropped, body: "" }).ok, false, "a removed ceiling counts as raised");
  assert.equal(sr.verdict({ base, head: dropped, body: "Size growth: the row moved" }).ok, true);
  const renamed = report({ "view.bytes": 5000 });
  renamed.metrics[0].id = "view.total_bytes";
  renamed.metrics[0].ceiling = null;
  const v = sr.verdict({ base, head: renamed, body: "" });
  assert.equal(v.ok, false, "a renamed metric is a number no longer reported");
  assert.ok(v.lines.some((l) => /no longer reported: Document view: bytes/.test(l)), v.lines.join("\n"));
  assert.equal(sr.verdict({ base: null, head: renamed, body: "" }).ok, true, "the first report has nothing to lose");
  const md = sr.renderComment({ base, head: renamed, baseSha: "a".repeat(40), headSha: "b".repeat(40), body: "" });
  assert.match(md, /No longer reported \(on main, not in this pull request\):\n\n- Document view: bytes: 1,000, ceiling 1,100/);
});

test("the change is absolute and in percent", () => {
  assert.equal(sr.change(1000, 1100), "+100 (+10.0%)");
  assert.equal(sr.change(2000, 1990), "−10 (−0.5%)");
  assert.equal(sr.change(5, 5), "±0 (0.0%)");
  assert.equal(sr.change(0, 3), "+3 (+new)");
  assert.equal(sr.change(1990307, 1990443), "+136 (+<0.1%)", "a nonzero change never reads as 0.0%");
  assert.equal(sr.change(2000, 1999), "−1 (−<0.1%)");
});

test("the comment shows main, this PR, the change and the ceiling", () => {
  const md = sr.renderComment({
    base: report(),
    head: report({ "view.bytes": 1050 }),
    baseSha: "a".repeat(40),
    headSha: "b".repeat(40),
    body: "Size growth: a bigger card",
  });
  assert.ok(md.startsWith(sr.COMMENT_MARKER));
  assert.match(md, /\| Document view: bytes \| 1,000 \| 1,050 \| \+50 \(\+5\.0%\) \| 1,100 \| ⚠️ grew \|/);
  assert.match(md, /main `aaaaaaa` · this PR `bbbbbbb`/);
  assert.match(md, /the body's `Size growth:` line gives the reason/);
  assert.match(md, /<summary>1 unchanged numbers without a ceiling<\/summary>/);
  assert.doesNotMatch(md, /a bigger card/, "the body's text is never copied into the comment");
  const first = sr.renderComment({ base: null, head: report(), baseSha: "x", headSha: "c".repeat(40), body: "" });
  assert.match(first, /\| Document view: bytes \| — \| 1,000 \| new \| 1,100 \|/);
  assert.match(first, /main `unknown`/);
});

function fakeGithub(files, comments = []) {
  const calls = [];
  const github = {
    rest: {
      repos: {
        getContent: async ({ ref }) => {
          if (!(ref in files)) throw Object.assign(new Error("Not Found"), { status: 404 });
          const content = Buffer.from(files[ref]).toString("base64");
          return { data: { type: "file", size: files[ref].length, content } };
        },
      },
      pulls: { get: async () => ({ data: { head: { sha: "h".repeat(40) }, base: { sha: "m".repeat(40) }, body: "" } }) },
      issues: {
        listComments: "listComments",
        createComment: async (a) => calls.push(["create", a]),
        updateComment: async (a) => calls.push(["update", a]),
      },
    },
    paginate: async () => comments,
  };
  return { github, calls };
}

function fakeCore() {
  const core = { failed: null, infos: [], warnings: [] };
  core.info = (m) => core.infos.push(m);
  core.warning = (m) => core.warnings.push(m);
  core.setFailed = (m) => (core.failed = m);
  core.summary = { addRaw() { return this; }, write: async () => {} };
  return core;
}

const context = (body = "") => ({
  repo: { owner: "o", repo: "r" },
  payload: { pull_request: { number: 7, body, head: { sha: "h".repeat(40) }, base: { sha: "m".repeat(40) } } },
});

test("the check fetches both reports through the API", async () => {
  const files = { ["m".repeat(40)]: JSON.stringify(report()), ["h".repeat(40)]: JSON.stringify(report({ "view.elements": 60 })) };
  let core = fakeCore();
  await sr.check({ github: fakeGithub(files).github, context: context(""), core });
  assert.match(core.failed, /Size growth/);
  core = fakeCore();
  await sr.check({ github: fakeGithub(files).github, context: context("Size growth: why"), core });
  assert.equal(core.failed, null);
  core = fakeCore();
  await sr.check({ github: fakeGithub({}).github, context: context(""), core });
  assert.match(core.failed, /missing/);
});

test("the comment is created once, then updated in place", async () => {
  const files = { ["m".repeat(40)]: JSON.stringify(report()), ["h".repeat(40)]: JSON.stringify(report()) };
  let f = fakeGithub(files, [{ id: 3, user: { login: "someone" }, body: sr.COMMENT_MARKER }]);
  await sr.publish({ github: f.github, context: context(), core: fakeCore() });
  assert.equal(f.calls.length, 1);
  assert.equal(f.calls[0][0], "create", "a comment by someone else is never edited");
  f = fakeGithub(files, [{ id: 9, user: { login: "github-actions[bot]" }, body: `${sr.COMMENT_MARKER}\nold` }]);
  await sr.publish({ github: f.github, context: context(), core: fakeCore() });
  assert.deepEqual(f.calls.map((c) => [c[0], c[1].comment_id]), [["update", 9]]);
});

test("a report that is not valid data is never echoed", async () => {
  const evil = JSON.stringify({ ...report(), metrics: [{ id: "x", label: "@everyone <script>", unit: "bytes", value: 1, ceiling: null }] });
  const f = fakeGithub({ ["h".repeat(40)]: evil });
  const core = fakeCore();
  await sr.publish({ github: f.github, context: context(), core });
  const body = f.calls[0][1].body;
  assert.match(body, /is not a valid size report/);
  assert.doesNotMatch(body, /everyone|script/);
});

test("an invalid base report is named, never shown as no base", async () => {
  const [baseSha, headSha] = ["b".repeat(40), "c".repeat(40)];
  const f = fakeGithub({ [baseSha]: "{", [headSha]: JSON.stringify(report()) });
  f.github.rest.pulls.get = async () => ({ data: { head: { sha: headSha }, base: { sha: baseSha }, body: "" } });
  await sr.publish({ github: f.github, context: context(), core: fakeCore() });
  const body = f.calls[0][1].body;
  assert.match(body, /at `bbbbbbb` is not a valid size report/);
  assert.doesNotMatch(body, /No number grew|no size report yet/);
});

test("an API failure fails the run and leaves the comment alone", async () => {
  const f = fakeGithub({ ["h".repeat(40)]: JSON.stringify(report()) });
  f.github.rest.repos.getContent = async () => {
    throw Object.assign(new Error("Server Error"), { status: 502 });
  };
  await assert.rejects(sr.publish({ github: f.github, context: context(), core: fakeCore() }), /Server Error/);
  assert.equal(f.calls.length, 0);
});

test("the size and perf checks load their script from the base, never the pull request", () => {
  for (const name of ["size.yml", "perf-table.yml"]) {
    const yml = fs.readFileSync(path.join(__dirname, "../workflows", name), "utf8");
    assert.match(yml, /ref: \$\{\{ github\.event\.pull_request\.base\.sha \}\}/, name);
    assert.doesNotMatch(yml, /pull_request\.head\.sha/, name);
  }
});
