"use strict";
// Runs the `surfaces` check's script out of .github/workflows/surfaces.yml (it has no checkout, so
// the script lives in the workflow) against made-up pull requests.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const yml = fs.readFileSync(path.join(__dirname, "../workflows/surfaces.yml"), "utf8");
const lines = yml.split("\n");
const at = lines.findIndex((l) => /^\s+script: \|$/.test(l));
const indent = lines[at + 1].match(/^ */)[0].length;
const body = [];
for (const l of lines.slice(at + 1)) {
  if (l.trim() && l.match(/^ */)[0].length < indent) break;
  body.push(l.slice(indent));
}
const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;
const script = new AsyncFunction("core", "context", body.join("\n"));

async function run(title, prBody) {
  const core = { failed: null, info() {}, setFailed: (m) => (core.failed = m) };
  await script(core, { payload: { pull_request: { title, body: prBody } } });
  return core.failed;
}

const ALL = [
  "## Surfaces",
  "",
  "- Web: changed",
  "- Hermes page: not affected: no page or CLI change",
  "- **App**: follows: needs a design answer from the maintainer",
  "- CLI: not affected: the CLI prints none of it",
  "- Agents: changed (the skill names the new flag)",
].join("\n");

test("feat, fix and perf need the section; other titles skip it", async () => {
  assert.match(await run("feat(web): avatars", "## What and why\n\nx"), /needs a ## Surfaces section/);
  assert.match(await run("fix: a crash", ""), /needs a ## Surfaces section/);
  assert.match(await run("perf!: smaller pages", ""), /Surfaces/);
  assert.equal(await run("docs: typo", ""), null);
  assert.equal(await run("ci: a check", ""), null);
  assert.equal(await run("chore(deps): bump", ""), null);
});

test("every surface has one line with an allowed answer", async () => {
  assert.equal(await run("feat: x", ALL), null);
  assert.match(await run("feat: x", ALL.replace("- CLI: not affected: the CLI prints none of it\n", "")), /CLI: missing/);
  assert.match(await run("feat: x", ALL.replace("not affected: no page or CLI change", "not affected:")), /Hermes page: "not affected:" needs the reason/);
  assert.match(await run("feat: x", ALL.replace("follows: needs a design answer from the maintainer", "follows: later")), /App: "follows:" is allowed only/);
  assert.equal(await run("fix: x", ALL.replace("follows: needs a design answer from the maintainer", "follows: urgent fix to a live server")), null);
  assert.equal(await run("fix: x", ALL.replace("follows: needs a design answer from the maintainer", "follows: an outside contributor's pull request")), null);
  assert.match(await run("feat: x", ALL.replace("- Web: changed", "- Web: maybe")), /Web: say "changed"/);
  assert.match(await run("feat: x", `${ALL}\n- Web: changed`), /Web: say it once/);
});

test("the template's hints and code blocks are not answers", async () => {
  const hinted = "## Surfaces\n\n<!-- - Web: changed\n- Hermes page: changed\n- App: changed\n- CLI: changed\n- Agents: changed -->";
  assert.match(await run("feat: x", hinted), /needs a ## Surfaces section/);
  assert.match(await run("feat: x", "## Surfaces\n\n```\n- Web: changed\n```\n- Hermes page: changed"), /Web: missing/);
  assert.match(await run("feat: x", ALL.replace("## Surfaces", "## Notes")), /needs a ## Surfaces section/);
});
