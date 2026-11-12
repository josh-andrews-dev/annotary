"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const pt = require("./perf-table.cjs");

const README = fs.readFileSync(path.join(__dirname, "../../perf/README.md"), "utf8");
const SHA = "0123456789abcdef0123456789abcdef01234567";

function table(sha = SHA, { fail = false, rows = 6, dirty = false } = {}) {
  const names = ["Document view FCP", "Document view interactive", "Version switch", "Document list", "CLS after first paint", "Optimistic reply (perceived)"];
  const lines = names.slice(0, rows).map((n, i) => `${n.padEnd(44)} 100 ms       80.0      78.0      96.0      ${fail && i === 1 ? "FAIL" : "pass"}`);
  return [
    "## Perf",
    "",
    "```",
    "perf: perf-plan k7q2m9xd — 5146 lines; release binary; 5 cold runs each",
    `commit: ${sha}${dirty ? " (dirty: uncommitted changes)" : ""}`,
    "",
    "metric                                       budget       median    min       max       result",
    "-".repeat(90),
    ...lines,
    "Server TTFB (Document view)                  (reported)   11.0      10.0      12.0      —",
    "```",
  ].join("\n");
}

const base = { fork: false, files: ["crates/annotary/src/server/ui/app.js"], readme: README, commits: ["f".repeat(40), SHA] };

test("the README lists the web view paths", () => {
  const globs = pt.webViewPaths(README);
  assert.ok(globs && globs.length >= 5, String(globs));
  assert.deepEqual(pt.touched(["crates/annotary/src/server/view.rs", "crates/renderer/src/lib.rs", "docs/x.md"], globs), [
    "crates/annotary/src/server/view.rs",
    "crates/renderer/src/lib.rs",
  ]);
});

test("globs", () => {
  assert.ok(pt.globToRegExp("crates/renderer/**").test("crates/renderer/src/a/b.rs"));
  assert.ok(pt.globToRegExp("perf/*").test("perf/BUDGETS.md"));
  assert.ok(!pt.globToRegExp("perf/*").test("perf/x/y"));
  assert.ok(!pt.globToRegExp("a.rs").test("aXrs"));
});

test("a web view change needs the table from a commit of the pull request, every budget passing", () => {
  assert.equal(pt.verdict({ ...base, body: table() }).ok, true);
  assert.equal(pt.verdict({ ...base, body: table(SHA.slice(0, 7)) }).ok, true, "a short SHA");
  assert.equal(pt.verdict({ ...base, body: "## Summary\nno table" }).ok, false);
  assert.equal(pt.verdict({ ...base, body: table("abcdef1") }).ok, false, "not a commit of the PR");
  assert.equal(pt.verdict({ ...base, body: table(SHA, { fail: true }) }).ok, false);
  assert.equal(pt.verdict({ ...base, body: table(SHA, { rows: 3 }) }).ok, false, "a partial run");
  assert.equal(pt.verdict({ ...base, body: table(SHA, { dirty: true }) }).ok, false);
  assert.equal(pt.verdict({ ...base, body: `<!--\n${table()}\n-->` }).ok, false, "a commented-out table");
  assert.equal(pt.verdict({ ...base, body: table().replace("## Perf", "## Evidence") }).ok, false, "under another heading");
});

test("other paths and forks need nothing", () => {
  assert.equal(pt.verdict({ ...base, files: ["docs/decisions.md"], body: "" }).ok, true);
  assert.equal(pt.verdict({ ...base, fork: true, body: "" }).ok, true);
  assert.equal(pt.verdict({ ...base, readme: "no list", body: table() }).ok, false, "a README without the list fails loudly");
});

test("the check reads files, commits and the base README through the API", async () => {
  const content = Buffer.from(README).toString("base64");
  const github = {
    rest: {
      pulls: { listFiles: "files", listCommits: "commits" },
      repos: { getContent: async () => ({ data: { type: "file", content } }) },
    },
    paginate: async (what) =>
      what === "files" ? [{ filename: "crates/annotary/src/server/splice.rs" }] : [{ sha: SHA }],
  };
  const run = async (body, headRepo = "o/r") => {
    const core = { failed: null, info() {}, setFailed: (m) => (core.failed = m), summary: { addRaw() { return this; }, write: async () => {} } };
    const context = { repo: { owner: "o", repo: "r" }, payload: { pull_request: { number: 4, body, head: { sha: SHA, repo: headRepo && { full_name: headRepo } }, base: { sha: "b".repeat(40) } } } };
    await pt.check({ github, context, core });
    return core.failed;
  };
  assert.equal(await run(table()), null);
  assert.match(await run("nothing"), /no `## Perf` section/);
  assert.equal(await run("nothing", "someone/fork"), null);
  assert.equal(await run("nothing", null), null, "a deleted fork");
});
