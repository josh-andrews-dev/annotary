// The size report's two pull request jobs (perf/README.md, "The size report"):
//   check   — the `size` check (size.yml, read-only): fails when a number in perf/size-report.json
//             is above its ceiling, or larger than on the base branch without a `Size growth:`
//             line in the pull request body;
//   publish — the comment (size-comment.yml): one comment per pull request, updated in place.
// Both read the two committed reports through the API and treat them strictly as data: every
// field is checked against a fixed shape before anything is compared or printed, so a pull
// request (a fork's included) can put nothing into the comment but numbers and fixed labels.
"use strict";

const REPORT_PATH = "perf/size-report.json";
const MAX_BYTES = 64 * 1024;
const MAX_METRICS = 200;
const MAX_VALUE = 1e12;
const COMMENT_MARKER = "<!-- annotary-size-report -->";
const ID = /^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$/;
// Labels end up in a Markdown table: no `|`, `<`, `@`, backticks, brackets or line breaks.
const LABEL = /^[A-Za-z0-9 ,.()/:'-]{1,80}$/;
const UNITS = ["bytes", "count"];
const FIXTURE_KEYS = ["scale", "seed", "lines", "versions", "threads", "replies", "list_documents"];

function fail(msg) {
  throw new Error(`${REPORT_PATH}: ${msg}`);
}

function isObject(v) {
  return v !== null && typeof v === "object" && !Array.isArray(v);
}

function exactKeys(v, keys, what) {
  if (!isObject(v)) fail(`${what} must be an object`);
  const have = Object.keys(v).sort();
  const want = [...keys].sort();
  if (have.length !== want.length || have.some((k, i) => k !== want[i])) fail(`${what} has unexpected fields`);
}

function count(v, what) {
  if (!Number.isSafeInteger(v) || v < 0 || v > MAX_VALUE) fail(`${what} must be a whole number from 0 to 10^12`);
}

/** Check a parsed report against the one shape the size test writes; returns it unchanged. */
function validateReport(r) {
  exactKeys(r, ["schema", "fixture", "metrics"], "the report");
  if (r.schema !== 1) fail("schema must be 1");
  exactKeys(r.fixture, FIXTURE_KEYS, "fixture");
  if (!["full", "smoke"].includes(r.fixture.scale)) fail("fixture.scale must be full or smoke");
  for (const k of FIXTURE_KEYS.slice(1)) count(r.fixture[k], `fixture.${k}`);
  if (!Array.isArray(r.metrics) || r.metrics.length === 0 || r.metrics.length > MAX_METRICS) {
    fail(`metrics must be a list of 1 to ${MAX_METRICS}`);
  }
  const seen = new Set();
  r.metrics.forEach((m, i) => {
    exactKeys(m, ["id", "label", "unit", "value", "ceiling"], `metrics[${i}]`);
    if (typeof m.id !== "string" || m.id.length > 64 || !ID.test(m.id)) fail(`metrics[${i}].id is not a metric id`);
    if (seen.has(m.id)) fail(`metrics[${i}].id repeats ${m.id}`);
    seen.add(m.id);
    if (typeof m.label !== "string" || !LABEL.test(m.label)) fail(`metrics[${i}].label has characters a label may not`);
    if (!UNITS.includes(m.unit)) fail(`metrics[${i}].unit must be bytes or count`);
    count(m.value, `metrics[${i}].value`);
    if (m.ceiling !== null) count(m.ceiling, `metrics[${i}].ceiling`);
  });
  return r;
}

/** Parse and validate the report's text (at most 64 KiB). */
function parseReport(text) {
  if (typeof text !== "string" || Buffer.byteLength(text, "utf8") > MAX_BYTES) fail("must be text under 64 KiB");
  let r;
  try {
    r = JSON.parse(text);
  } catch {
    fail("is not JSON");
  }
  return validateReport(r);
}

/**
 * The prose of a pull request body: HTML comments (the template's hints) and fenced code removed,
 * as the `lessons` check reads it. Returns the remaining lines.
 */
function proseLines(body) {
  const text = String(body || "").replace(/\r/g, "").replace(/<!--[\s\S]*?(-->|$)/g, "");
  const out = [];
  let fence = null;
  for (const line of text.split("\n")) {
    const m = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (fence) {
      if (m && m[1][0] === fence[0] && m[1].length >= fence.length && /^\s*[`~]+\s*$/.test(line)) fence = null;
      continue;
    }
    if (m) {
      fence = m[1];
      continue;
    }
    out.push(line);
  }
  return out;
}

/** The reason on the body's `Size growth:` line, or null when there is none. */
function growthReason(body) {
  for (const line of proseLines(body)) {
    const m = /^\s{0,3}(?:[-*+]\s+)?(?:\*\*|__)?size growth(?:\*\*|__)?\s*:(?:\*\*|__)?\s*(.*)$/i.exec(line);
    if (m && /[A-Za-z0-9].*[A-Za-z0-9]/.test(m[1].replace(/[*_`]/g, ""))) return m[1].trim();
  }
  return null;
}

/** One row per metric of the head report, against the base report when there is one. */
function compare(base, head) {
  const baseById = new Map((base ? base.metrics : []).map((m) => [m.id, m]));
  return head.metrics.map((m) => {
    const b = baseById.get(m.id);
    const was = b ? b.value : null;
    const over = m.ceiling !== null && m.value > m.ceiling;
    const grew = was !== null && m.value > was;
    // Dropping a ceiling raises it to nothing: it needs a reason like any raise.
    const ceilingRaised =
      b !== undefined && b.ceiling !== null && (m.ceiling === null || m.ceiling > b.ceiling);
    return { id: m.id, label: m.label, unit: m.unit, was, now: m.value, ceiling: m.ceiling, over, grew, ceilingRaised };
  });
}

/** Base metrics the head no longer reports (removed or renamed): unguarded from now on. */
function removed(base, head) {
  if (!base) return [];
  const ids = new Set(head.metrics.map((m) => m.id));
  return base.metrics.filter((m) => !ids.has(m.id));
}

/** The `size` check's verdict: `{ ok, lines }` (lines go to the log and the job summary). */
function verdict({ base, head, body }) {
  const rows = compare(base, head);
  const reason = growthReason(body);
  const lines = [];
  const over = rows.filter((r) => r.over);
  const grown = rows.filter((r) => r.grew || r.ceilingRaised);
  const gone = removed(base, head);
  for (const r of over) lines.push(`above its ceiling: ${r.label} (${r.id}) ${fmt(r.now)} > ${fmt(r.ceiling)}`);
  for (const r of grown) {
    lines.push(
      r.grew
        ? `grew: ${r.label} (${r.id}) ${fmt(r.was)} -> ${fmt(r.now)} (${change(r.was, r.now)})`
        : `ceiling ${r.ceiling === null ? "removed" : "raised"}: ${r.label} (${r.id}) to ${fmt(r.ceiling)}`,
    );
  }
  for (const m of gone) lines.push(`no longer reported: ${m.label} (${m.id}), ${fmt(m.value)} on the base`);
  if (!base) lines.push("The base branch has no size report yet: only the ceilings apply.");
  let ok = over.length === 0;
  if (over.length) lines.push("A number above its ceiling always fails: make it smaller, or raise the ceiling by the budget-raise rule in perf/BUDGETS.md.");
  if (grown.length || gone.length) {
    if (reason) lines.push(`Growth accepted by the body's "Size growth:" line.`);
    else {
      ok = false;
      lines.push('Growth, a raised or removed ceiling, or a number no longer reported needs a "Size growth:" line in the pull request body saying why; edit the body and the check runs again.');
    }
  }
  if (ok && !over.length && !grown.length && !gone.length) lines.push("No number grew, and none is above its ceiling.");
  return { ok, lines, rows, reason, gone };
}

function fmt(n) {
  return n === null || n === undefined ? "—" : n.toLocaleString("en-US");
}

function change(was, now) {
  if (was === null) return "new";
  const d = now - was;
  const sign = d > 0 ? "+" : d < 0 ? "−" : "±";
  const pct = was === 0 ? (d === 0 ? "0.0%" : "new") : `${(Math.abs(d) / was * 100).toFixed(1)}%`;
  return `${sign}${Math.abs(d).toLocaleString("en-US")} (${sign === "±" ? "" : sign}${pct})`;
}

function sha7(sha) {
  return /^[0-9a-f]{40}$/.test(sha || "") ? sha.slice(0, 7) : "unknown";
}

function row(r) {
  const mark = r.over ? "❌ over" : r.grew ? "⚠️ grew" : r.ceilingRaised ? "⚠️ ceiling" : "";
  return `| ${r.label} | ${fmt(r.was)} | ${fmt(r.now)} | ${r.was === null ? "new" : change(r.was, r.now)} | ${fmt(r.ceiling)} | ${mark} |`;
}

/** The comment's Markdown. Only validated numbers, labels and SHAs reach it. */
function renderComment({ base, head, baseSha, headSha, body, problem }) {
  const out = [COMMENT_MARKER, "## Size report", ""];
  if (problem) {
    out.push(`❌ ${problem}`, "", "_Updated in place by a workflow from the default branch._");
    return out.join("\n");
  }
  const v = verdict({ base, head, body });
  const over = v.rows.filter((r) => r.over).length;
  const grown = v.rows.filter((r) => r.grew || r.ceilingRaised).length + v.gone.length;
  if (over) out.push(`❌ ${over} number${over > 1 ? "s are" : " is"} above the ceiling: the \`size\` check fails.`);
  else if (grown && v.reason) out.push(`⚠️ ${grown} number${grown > 1 ? "s" : ""} grew; the body's \`Size growth:\` line gives the reason.`);
  else if (grown) out.push(`❌ ${grown} number${grown > 1 ? "s" : ""} grew with no \`Size growth:\` line in the body: the \`size\` check fails.`);
  else out.push("✅ No number grew, and none is above its ceiling.");
  if (!base) out.push("", "> The base branch has no size report yet, so there is nothing to compare with.");
  const fx = head.fixture;
  if (base && FIXTURE_KEYS.some((k) => base.fixture[k] !== fx[k])) {
    out.push("", "> The fixture changed between the two reports, so the changes are partly the fixture's.");
  }
  const header = ["| Metric | main | this PR | Change | Ceiling | |", "| --- | ---: | ---: | ---: | ---: | --- |"];
  const shown = v.rows.filter((r) => r.ceiling !== null || r.grew || r.over || (r.was !== null && r.was !== r.now));
  const rest = v.rows.filter((r) => !shown.includes(r));
  out.push("", ...header, ...shown.map(row));
  if (v.gone.length) {
    out.push("", "No longer reported (on main, not in this pull request):", "", ...v.gone.map((m) => `- ${m.label}: ${fmt(m.value)}${m.ceiling === null ? "" : `, ceiling ${fmt(m.ceiling)}`}`));
  }
  if (rest.length) {
    out.push("", "<details>", `<summary>${rest.length} unchanged numbers without a ceiling</summary>`, "", ...header, ...rest.map(row), "", "</details>");
  }
  out.push(
    "",
    `main \`${sha7(baseSha)}\` · this PR \`${sha7(headSha)}\` · fixture: ${fx.scale} scale, seed ${fx.seed}, ${fmt(fx.lines)} lines, ${fmt(fx.threads)} Threads, ${fmt(fx.replies)} Replies · bytes are uncompressed; how each number is counted: \`docs/dev/perf-suite.md\``,
    "",
    "_Updated in place by a workflow from the default branch. Both reports are read through the API and validated as data; no pull request code runs._",
  );
  return out.join("\n");
}

/** `perf/size-report.json` at `ref`: the parsed report, `null` when the file is not there. */
async function fetchReport(github, owner, repo, ref) {
  let data;
  try {
    ({ data } = await github.rest.repos.getContent({ owner, repo, path: REPORT_PATH, ref }));
  } catch (e) {
    if (e.status === 404) return null;
    throw e;
  }
  if (!isObject(data) || data.type !== "file" || typeof data.content !== "string" || data.size > MAX_BYTES) {
    fail("is not a regular file under 64 KiB");
  }
  return parseReport(Buffer.from(data.content, "base64").toString("utf8"));
}

function pullOf(context) {
  const pr = context.payload.pull_request;
  if (!pr || !Number.isSafeInteger(pr.number) || pr.number <= 0) throw new Error("not a pull request event");
  return pr;
}

/** The `size` check (`pull_request`, read-only token). */
async function check({ github, context, core }) {
  const pr = pullOf(context);
  const { owner, repo } = context.repo;
  const head = await fetchReport(github, owner, repo, pr.head.sha);
  if (!head) {
    core.setFailed(`${REPORT_PATH} is missing at ${sha7(pr.head.sha)}: run \`make size-report\` and commit it.`);
    return;
  }
  const base = await fetchReport(github, owner, repo, pr.base.sha);
  const v = verdict({ base, head, body: pr.body });
  for (const l of v.lines) core.info(l);
  await core.summary.addRaw(v.lines.map((l) => `- ${l}`).join("\n")).write();
  if (!v.ok) core.setFailed(v.lines.join("\n"));
}

async function upsertComment(github, owner, repo, number, body) {
  const comments = await github.paginate(github.rest.issues.listComments, { owner, repo, issue_number: number, per_page: 100 });
  const mine = comments.find((c) => c.user && c.user.login === "github-actions[bot]" && (c.body || "").startsWith(COMMENT_MARKER));
  if (mine) {
    if (mine.body !== body) await github.rest.issues.updateComment({ owner, repo, comment_id: mine.id, body });
  } else {
    await github.rest.issues.createComment({ owner, repo, issue_number: number, body });
  }
}

/** The comment (`pull_request_target`; this file comes from the default branch). */
async function publish({ github, context, core }) {
  const pr = pullOf(context);
  const { owner, repo } = context.repo;
  // The newest head: an older run's numbers never overwrite a newer one's.
  const { data: now } = await github.rest.pulls.get({ owner, repo, pull_number: pr.number });
  const headSha = now.head.sha;
  const baseSha = now.base.sha;
  let body;
  try {
    const head = await fetchReport(github, owner, repo, headSha);
    const base = head ? await fetchReport(github, owner, repo, baseSha).catch(() => null) : null;
    body = head
      ? renderComment({ base, head, baseSha, headSha, body: now.body })
      : renderComment({ problem: `This pull request has no \`${REPORT_PATH}\` at \`${sha7(headSha)}\`.` });
  } catch (e) {
    core.warning(String(e.message || e));
    body = renderComment({ problem: `\`${REPORT_PATH}\` at \`${sha7(headSha)}\` is not a valid size report.` });
  }
  await upsertComment(github, owner, repo, pr.number, body);
  core.info(`Size report comment on #${pr.number} is up to date.`);
}

module.exports = {
  COMMENT_MARKER,
  REPORT_PATH,
  change,
  check,
  compare,
  fetchReport,
  removed,
  growthReason,
  parseReport,
  proseLines,
  publish,
  renderComment,
  validateReport,
  verdict,
};
