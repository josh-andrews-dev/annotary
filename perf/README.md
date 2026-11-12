# The perf suite (`make perf`, ticket 15)

Budgets as tests: `make perf` builds the **release** binaries, spawns a real `annotary serve`
on a temp HOME, seeds it with the generated fixture through the real binary, measures the web
UI with Playwright + CDP (cold browser cache per run, median of 5) and fails on any budget miss.
The budgets — FCP < 100 ms, interactive < 200 ms, Version switch < 150 ms, Document list
< 100 ms, CLS 0 after first paint, optimistic reply < 50 ms perceived — live in
`BUDGETS.md` **and nowhere else** (the runner parses that table); server TTFB is
reported separately. Absolute values, no CI multiplier.

## What is in here

| file | what |
|---|---|
| `BUDGETS.md` | the budgets (the single source), how each is measured, the budget-raise rule, the record of every raise and the optimization log |
| `budgets.ts` | reads the table of `BUDGETS.md` for the runner |
| `playwright.config.ts` | one worker, no retries, no parallelism; `global-setup.ts` / `global-teardown.ts` |
| `global-setup.ts` | spawns the release server (`tests/web/harness.ts`'s `spawnServer`) and runs `perfgen seed` — the generated Document (≥ 5,000 lines / ~50k words, 5 Versions, ≥ 500 Threads with mixed placements, Replies, mentions, Actions on two Versions) plus 60 small Documents for the list; state in `target/perf/harness.json` |
| `perf.spec.ts` | the runner: Document view (FCP, interactive, CLS, TTFB; layout A at 1280 px, layout C at 600 px reported), Version switch, Document list, optimistic reply; prints the table, writes `target/perf/results.{md,json}`, asserts the budgets |
| `../crates/perfgen` | the deterministic generator (`Plan::generate(scale, seed)`) and the seeder (`seed()`); `perfgen stats|write|seed` |

The generator is checked in; its output is not — every run regenerates the fixture from the
seed (`PERF_SEED`, default 1). The smoke variant (`crates/annotary/tests/perf_smoke.rs`) runs
the generator at `Scale::SMOKE` through the real binary under plain `cargo test`, so the
generator and the seeder cannot rot between perf runs.

## Running it

```
make perf                      # release build → server + fixture → Playwright → table
PERF_RUNS=3 make perf          # fewer runs while iterating (the record is 5)
PERF_SCALE=smoke make perf     # the small fixture (the numbers are not the budgets' numbers)
CD_PERF_KEEP=1 make perf       # keep the seeded temp store / HOME after the run
npx playwright test -c perf/playwright.config.ts   # after `cargo build --release -p annotary -p annotary-perfgen`
```

A run takes about 25 s on a recent Mac (≈ 2 s of it seeding). The table `make perf` prints is
the record of the run; `target/perf/results.md` keeps it.

`make perf` is part of `make check` (ticket 16), the definition of done
(`docs/PHASE1-DONE.md`); `PERF=0 make check` / `make check-ci` leave it out where the release
toolchain / Chromium is not. It is not part of this repo's CI (see "Where it runs").

The table ends with the commit it ran on (`commit: SHA`, with `(dirty: …)` when the checkout had
uncommitted changes), so a pull request can show which code it measured.

## The web view paths

A change to any of these is a web view change: its pull request needs a `make perf` table (below).
The `perf` check reads this list from the base branch; keep it between the two markers.

<!-- web-view-paths -->
- `crates/annotary/src/server/view.rs`
- `crates/annotary/src/server/splice.rs`
- `crates/annotary/src/server/listing.rs`
- `crates/annotary/src/server/ui.rs`
- `crates/annotary/src/server/prefs.rs`
- `crates/annotary/src/server/render.rs`
- `crates/annotary/src/server/ui/app.js`
- `crates/annotary/src/server/ui/app.css`
- `crates/renderer/**`
- `crates/store/src/docs.rs`
- `crates/store/src/threads.rs`
- `crates/store/src/render.rs`
- `crates/perfgen/**`
- `perf/*.ts`
<!-- /web-view-paths -->

## Budgets and the budget-raise rule

Budgets may change only by the rule in `BUDGETS.md`: at least three genuine, measured
optimization attempts at the missed budget first; then that budget alone may be raised to one
absolute value with the attempts and their numbers recorded there; a raised budget is still
asserted. Today no budget is raised — the original values hold on the fixture (see the
optimization log in `BUDGETS.md` for what it took).

## Where it runs

**Never on a GitHub-hosted runner**: the budgets are absolute and the numbers of a shared, noisy
runner mean nothing, so no workflow in this repo runs `make perf`. Nor does a pull request from a fork
run it anywhere: this repo has no self-hosted runner, because one would run a stranger's code on
someone's machine.

**The budgets are Mac numbers.** They were set on an Apple-silicon Mac, and they hold there. A Linux
host does not reproduce them: on 2026-11-11 one at `3075baa`, on unchanged code, measured Document
view interactive at 250 ms against the 200 ms budget (176 to 358 ms over five runs) and first paint
exactly at the 100 ms budget. A run on Linux is useful for finding a regression, never as the
record.

**Before a web view change merges** (a path in the list above), the maintainer's session runs `make
perf` on a Mac, at a commit of the pull request, from a clean checkout, one run at a time on that
machine, and pastes the table it prints under a `## Perf` heading in the pull request body. The
`perf` check (`.github/workflows/perf-table.yml`) fails such a pull request until that section holds
a table with no `FAIL` row whose `commit:` line names a commit of the pull request. A miss on a busy
machine is usually the machine: run it once more before looking for a regression. **A fork's pull
request is exempt**: the maintainer runs `make perf` on it before deploying, as with `make
verify-live`.

**The nightly run.** A dedicated Mac the maintainer keeps runs it against `main` every night, from a
job outside this repo, and files an issue here when a budget fails. That Mac also does other work, so a run can be slower than on a
developer's Mac; check that first when a budget fails there.

What a run needs, which is all you need to run it on your own Mac: rustup stable and Node 22 for the
user, nothing system-wide; then, from a clean checkout:

```sh
npm ci --no-fund --no-audit
npx playwright install chromium
make perf
```

`make perf` builds what it needs (`cargo build --release -p annotary -p annotary-perfgen`), spawns its
own server on an ephemeral port and cleans up after itself; nothing persists between runs except the
rustup, cargo, npm and Playwright caches. Only one run at a time on a machine: a concurrent run would
skew the numbers.

## The size report

Times need a quiet Mac; sizes do not. `crates/annotary/tests/size_report.rs` seeds the same
generated fixture into an in-process server and counts what the server sends: bytes (split into
inline script, page data, inline style, Document HTML and Thread cards), elements, requests before
first paint and database statements for the Document view, a Version page and the list; bytes and
statements for the API the iPhone and iPad app reads; the embed shell and the parts a Document
arrives in; and the card fragment and live event of one reply. The counts are the same on every
machine, so CI checks them on every pull request. `docs/dev/perf-suite.md` says how each is counted.

- `perf/size-report.json` is committed. `make size-check` (and the `cargo test` jobs on Linux and
  macOS) fails when it differs from a fresh measurement or a number is above its ceiling in
  `BUDGETS.md`; `make size-report` rewrites it. Commit the new file with the change that moved the
  numbers, and read its diff.
- The `size` check (`.github/workflows/size.yml`) compares the file at the base and at the head of a
  pull request. It fails when any number is larger, a ceiling is raised or removed, or a number is no
  longer reported, unless the pull request body has a line `Size growth: <why>`; it always fails
  above a ceiling. It runs the base branch's copy of `.github/scripts/size-report.cjs`, so a
  pull request cannot loosen it in the same change; a change to the report's shape lands in two
  steps, a script that accepts both shapes first. Editing the body runs it again.
- A comment on the pull request (`.github/workflows/size-comment.yml`) shows each number on `main`,
  on the pull request, the change and the ceiling, and is updated in place. It runs from the default
  branch and reads the two committed files through the API as data; it never checks out or runs the
  pull request's code, so a fork's pull request gets it too.
- Two open pull requests that both move a number conflict in `perf/size-report.json`; the later one
  runs `make size-report` again after merging `main`.
- annotary carries the original of `.github/scripts/size-report.cjs` (and of the `surfaces` workflow,
  `.github/workflows/surfaces.yml`). annotary-hermes and the iPhone and iPad app's repo each keep a
  copy, with their own fixture and report, and each repo's `size` check runs only its own copy. A fix
  to the script or the workflow here is ported to both copies in the same run, as "Hit every surface"
  says (`docs/dev/surfaces.md`).

## Reading a failure

The last test prints the table, then asserts. A `FAIL` row names the metric, its median and the
budget; `target/perf/results.json` has every sample. First check the machine (other load, a
debug build — `make perf` always builds release, but `ANNOTARY_BIN` can override), then the
change: the optimization log in `BUDGETS.md` lists what each budget depends on. Raising a budget
is the last resort and follows the rule.

A failure from a scheduled job needs one more check, because how the suite is started changes the
numbers. On macOS, launchd throttles a job whose `ProcessType` is unset, and Chromium inherits that.
On one Mac, at one commit and the same machine load (about 88% idle), a job like that failed 3 runs
of 3 (FCP 104 – 136 ms, optimistic reply medians 31 – 111 ms and unstable from run to run) while `make perf`
from an interactive shell passed 3 of 3 (FCP 76 ms, optimistic reply 14 – 18 ms). `taskpolicy -c utility make
perf` reproduced the failure in that shell. So before looking for a regression, run the same commit
from a shell on the same machine: a pass there points at the job's scheduling and not the code.
