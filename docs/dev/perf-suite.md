# Perf suite (ticket 15 — extend, don't rewrite)

Implementation notes, moved from the root `CLAUDE.md` on 2026-10-18 (read on demand; the
index is [README.md](README.md)). Extend them with the code they describe.

- **Files:** `crates/perfgen/` (`plan.rs`: `Scale {lines, words, threads, versions, images, list_docs}` with `Scale::FULL` (5,200 / 50k / 520 / 5 / 4 / 60) and `Scale::SMOKE`; `Plan {name, handles, versions: Vec<String>, images: Vec<Image{reference, width, height, png}>, steps: Vec<Step>, list_docs, stats: Stats}`; `Step::{Push, Thread{by, subject: Span{range, exact}|Document|Asset{reference}, body}, Reply, Resolve, Reopen, Delete, Act}`; the Document is a list of typed lines (heading / paragraph / item / quote / fence / code / table row / image / blank), each Version an edit of the model — targeted edits keep the statuses mixed: a word inserted *inside* a known span ⇒ refined, its line deleted ⇒ orphaned, inserts / deletes / touches / whitespace elsewhere ⇒ anchored, the last edit drops one image (its Asset Thread orphans); bodies stay under the 100k cap (asserted); `plan::extract(text, range)` checks ranges; `seed.rs`: `Target {bin, server, owner_token, home, work}` → `seed()` = `member add` ×2 + `push` per Version through the real binary, Threads / Replies / resolves / reopens / tombstones / Actions through `/api/v1` (ureq), the list Documents, then `GET …/threads?status=all&hidden=1` → `Seeded {doc_id, head, threads, statuses {anchored, refined, orphaned, document}, resolved, replies, list_ids, tokens, pushes_ms, elapsed_ms, stats}`), `perf/` (see the workspace map), `tests/perf_smoke.rs`.
- **Budgets** (`perf/BUDGETS.md`, parsed by `perf/budgets.ts` — ids `fcp interactive switch list cls optimistic`): Document view FCP 100 ms · interactive 200 ms (max(DOMContentLoaded, `cd:ready`) pushed past a running long task) · Version switch 150 ms (in-page click → FCP of `/v/{n−1}`) · Document list 100 ms · CLS 0 after first paint · optimistic reply 50 ms (Post click → next painted frame with the pending Comment, the server's answer held by `page.route`). Reported: TTFB per page, DCL, bytes, layout C. Measured on the fixture: `perf/BUDGETS.md`, "Measured" — all original budgets hold; none raised (`### Raised budgets` in `BUDGETS.md` is the record if one ever is).
- **The budget-raise rule** (human-authorized): ≥ 3 genuine, measured optimization attempts at a missed budget, then that budget alone may become one absolute value, recorded in `perf/BUDGETS.md` (original, new, the attempts with numbers, why the gap is not cheaply closable), still asserted. Budgets change nowhere else — not in code, not in a test.
- **Decisions to keep:** the suite is release-only and local (a developer's or a dedicated machine; never a shared runner; the budgets are read on a Mac: on the shared Linux host `main` itself misses Document view interactive under other sessions' load, 2026-11-08); the fixture is regenerated from the seed every run (nothing generated is committed); seeding goes through the real `push` (render + re-anchor at the edge) and the real API; cold browser cache per run, median of 5, absolute budgets; the Document view keeps Threads and placements in the first HTML — viewport-first rendering is CSS over the same DOM, not a second payload; the smoke test is the rot guard for the generator, `make perf` the one for the numbers. The seeder's `/api/v1` client (`seed.rs::Http`) opens a fresh connection per request (no idle pool, `Connection: close`) and retries a transport (`io`) failure, at most 3 attempts with 50 / 100 ms backoff; an HTTP status is never retried (CMD-10: under load a kept-alive socket the server had dropped failed the next request, `io: Invalid argument (os error 22)` on macOS, because XNU's `setsockopt` returns EINVAL on a socket shut down both ways). The retry can, in principle, repeat a non-idempotent POST whose answer was lost; `perf_smoke`'s exact counts would then fail loudly rather than silently.
- **Comparing two builds:** separate `make perf` runs on a loaded host differ by more than a CSS or script change does, in either direction; serve both builds at once and alternate cold loads of each (40 is enough) in one Playwright process, so the load falls on both.
- **A baseline on the same host:** give the other worktree its own `node_modules`, because `make perf` runs `npm ci`, which replaces a symlinked `node_modules` and empties the folder it pointed to.
## The size report

Deterministic counts of what the server sends, checked on every pull request (`perf/README.md`, "The size
report"). Files: `crates/annotary/tests/size_report.rs` (`#[ignore]`d; `make size-check`, `make
size-report`), `perf/size-report.json`, the "Size ceilings" table of `perf/BUDGETS.md`,
`crates/store/src/stmt_count.rs` (the `stmt-count` feature, on only through the `annotary` crate's
dev-dependencies), `.github/scripts/size-report.cjs` with its test, `.github/workflows/size.yml` and
`size-comment.yml`.

- **The fixture:** `Scale::FULL`, seed 1, seeded by `perfgen::seed` through the debug binary into a
  `TestServer` (no password). A browser's session comes from the first `GET /` on loopback; pages are
  requested with that cookie and `Accept: text/html` and no `cd_tz` cookie (a first visit); the API and
  embed mode with the owner's Bearer Token.
- **What is counted:** the Document view (`/d/{id}`), the previous Version (`/d/{id}/v/{head-1}`) and the
  list (`/`); `GET /api/v1/docs`, `…/versions/{head}/html`, `…/threads?status=all`; `GET /embed/shell` and
  the embedded Document view (every part fetched and joined; `parts` is `embed::cuts` of the normalized body at `PART_MAX`, which the test checks is within one of the server's `X-Annotary-Parts`, so this run's address and clock cannot move it); then one
  reply to the first open anchored Thread, the card fragment `GET /d/{id}/t/{tid}` and the SSE frame of
  its `thread.replied` event (`id:`, `event:`, `data:`, the blank line). Bytes are the body, uncompressed,
  without headers.
- **A page's parts** (`read_page`, a small tag scanner): inline script = executable `<script>` elements,
  page data = `<script type="application/json">` (`#cd-data`), inline style = `<style>`, Thread cards =
  the outermost `.cd-strip` elements, Document HTML = `article#cd-doc` without the cards, the rest = the
  header, the shell and everything else. They add up to the page. Elements = start tags (SVG children
  included, the doctype and comments not). Requests before first paint = 1 (the page) + stylesheets +
  classic scripts with `src` and no `async` / `defer` + CSS `@import`.
- **Database statements:** every statement SQLite runs on the store's connection (rusqlite's
  `trace_v2`, `BEGIN` and `COMMIT` included, triggers not). Each answer is requested once to warm the
  caches (rendered titles), then until two requests in a row agree, so a stray request to the test
  server cannot land in a count.
- **Normalized before counting:** the server's address (`http://127.0.0.1:PORT`, and percent-encoded in
  "Open in Annotary") becomes `http://127.0.0.1:7377`, and the text of every `<time class="cd-rel">`
  becomes `now`: the seed takes seconds to minutes, and `now` turns into `1m` after a minute. Everything
  else (ids, ULIDs, timestamps, hashes) has a fixed length. A run with a 70 s pause after seeding gave
  the same numbers.
- **Decisions to keep:** counts, never times; the full fixture, not the smoke one, so the numbers are the
  perf suite's page; the report is committed (the change shows in the diff, and a fork's pull request
  needs no artifact); ceilings live in `BUDGETS.md` and are copied into the report, so the comment and the
  `size` check read one file. The `size` and `perf` checks load their scripts from the pull request's base
  (`ref: base.sha`), never its head, so a pull request cannot loosen a check in the change it is checked
  on; `make test` runs the pull request's own copy's tests. The comment loads its script from the default
  branch. A missing base report means "no base"; an invalid one is named, and an API failure fails the run
  and leaves the last comment as it was.
<!-- ---- end ticket 15 ---- -->
<!-- ---- ticket 16 ---- -->
