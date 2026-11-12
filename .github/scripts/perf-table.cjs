// The `perf` check (perf-table.yml, read-only): a pull request from this repo (not a fork) that
// touches a web view path listed in perf/README.md needs a `## Perf` section in its body holding
// the table `make perf` printed, with no FAIL row, from a commit that is in the pull request.
// A fork's pull request is exempt: the maintainer runs `make perf` before deploying it.
"use strict";

const BEGIN = "<!-- web-view-paths -->";
const END = "<!-- /web-view-paths -->";
// The six timed budgets of perf/BUDGETS.md: a full run prints a `pass` row for each.
const BUDGET_ROWS = 6;

/** The web view paths listed between the markers in perf/README.md, as `- \`glob\`` lines. */
function webViewPaths(readme) {
  const text = String(readme || "");
  const a = text.indexOf(BEGIN);
  const b = text.indexOf(END);
  if (a < 0 || b < a) return null;
  const globs = [];
  for (const line of text.slice(a + BEGIN.length, b).split("\n")) {
    const m = /^\s*[-*]\s+`([^`]+)`/.exec(line);
    if (m) globs.push(m[1]);
  }
  return globs.length ? globs : null;
}

/** `*` matches inside one path segment, `**` across segments. */
function globToRegExp(glob) {
  let re = "";
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === "*" && glob[i + 1] === "*") {
      re += ".*";
      i++;
    } else if (c === "*") re += "[^/]*";
    else re += c.replace(/[.+?^${}()|[\]\\]/g, "\\$&");
  }
  return new RegExp(`^${re}$`);
}

function touched(files, globs) {
  const res = globs.map(globToRegExp);
  return files.filter((f) => res.some((r) => r.test(f)));
}

/** The lines of the body's `## Perf` section, fenced code included (the table is usually fenced). */
function perfSection(body) {
  const text = String(body || "").replace(/\r/g, "").replace(/<!--[\s\S]*?(-->|$)/g, "");
  const out = [];
  let inside = false;
  let fence = null;
  for (const line of text.split("\n")) {
    const f = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
      if (inside) out.push(line);
      continue;
    }
    if (f) {
      fence = f[1];
      if (inside) out.push(line);
      continue;
    }
    const h = /^ {0,3}(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
    if (h && h[1].length <= 2) {
      inside = h[1].length === 2 && h[2].toLowerCase() === "perf";
      continue;
    }
    if (inside) out.push(line);
  }
  return out;
}

/** The verdict on a pull request: `{ ok, lines }`. */
function verdict({ fork, files, readme, body, commits }) {
  if (fork) return { ok: true, lines: ["A fork's pull request is exempt: the maintainer runs `make perf` before deploying it."] };
  const globs = webViewPaths(readme);
  if (!globs) return { ok: false, lines: [`perf/README.md lists no web view paths between ${BEGIN} and ${END}.`] };
  const hit = touched(files, globs);
  if (!hit.length) return { ok: true, lines: ["No web view path changed: no timed run needed."] };
  const lines = [`Web view paths changed: ${hit.slice(0, 10).join(", ")}${hit.length > 10 ? ", …" : ""}`];
  const how = "Run `make perf` on a Mac at a commit of this pull request and paste its table, with the `commit:` line, under `## Perf` in the body (perf/README.md).";
  const section = perfSection(body);
  if (!section.length) return { ok: false, lines: [...lines, "The body has no `## Perf` section.", how] };
  const commitLine = section.find((l) => /^\s*commit:\s/i.test(l));
  const m = commitLine && /^\s*commit:\s+([0-9a-f]{7,40})\b(.*)$/i.exec(commitLine);
  if (!m) return { ok: false, lines: [...lines, "The `## Perf` section has no `commit: SHA` line from `make perf`.", how] };
  if (/dirty/i.test(m[2])) return { ok: false, lines: [...lines, "That run was made with uncommitted changes; run it again on a clean checkout.", how] };
  const sha = m[1].toLowerCase();
  if (!commits.some((c) => c.toLowerCase().startsWith(sha))) {
    return { ok: false, lines: [...lines, `The run's commit ${sha.slice(0, 12)} is not a commit of this pull request.`, how] };
  }
  const rows = section.filter((l) => /\s(pass|FAIL)\s*\|?\s*$/.test(l));
  const failed = rows.filter((l) => /\sFAIL\s*\|?\s*$/.test(l));
  if (failed.length) return { ok: false, lines: [...lines, `The table has ${failed.length} FAIL row${failed.length > 1 ? "s" : ""}: a budget is missed.`] };
  const passed = rows.length;
  if (passed < BUDGET_ROWS) {
    return { ok: false, lines: [...lines, `The table has ${passed} budget row${passed === 1 ? "" : "s"} marked pass; a full run prints ${BUDGET_ROWS}.`, how] };
  }
  return { ok: true, lines: [...lines, `The \`make perf\` table of ${sha.slice(0, 12)} passes every budget.`] };
}

async function readme(github, owner, repo, ref) {
  try {
    const { data } = await github.rest.repos.getContent({ owner, repo, path: "perf/README.md", ref });
    return data && data.type === "file" && typeof data.content === "string"
      ? Buffer.from(data.content, "base64").toString("utf8")
      : null;
  } catch (e) {
    if (e.status === 404) return null;
    throw e;
  }
}

/** The check, in actions/github-script. */
async function check({ github, context, core }) {
  const pr = context.payload.pull_request;
  const { owner, repo } = context.repo;
  const fork = !pr.head.repo || pr.head.repo.full_name !== `${owner}/${repo}`;
  let files = [];
  let commits = [];
  let text = null;
  if (!fork) {
    const list = await github.paginate(github.rest.pulls.listFiles, { owner, repo, pull_number: pr.number, per_page: 100 });
    files = list.flatMap((f) => [f.filename, f.previous_filename].filter(Boolean));
    const cs = await github.paginate(github.rest.pulls.listCommits, { owner, repo, pull_number: pr.number, per_page: 100 });
    commits = cs.map((c) => c.sha);
    // The base branch's list, so a pull request cannot drop its own paths; the head's when the
    // base has none yet.
    text = await readme(github, owner, repo, pr.base.sha);
    if (!webViewPaths(text)) text = await readme(github, owner, repo, pr.head.sha);
  }
  const v = verdict({ fork, files, readme: text, body: pr.body, commits });
  for (const l of v.lines) core.info(l);
  await core.summary.addRaw(v.lines.map((l) => `- ${l}`).join("\n")).write();
  if (!v.ok) core.setFailed(v.lines.join("\n"));
}

module.exports = { check, globToRegExp, perfSection, touched, verdict, webViewPaths };
