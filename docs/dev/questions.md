# Questions (extend, don't rewrite)

A Question is a choice put to a Document's readers (glossary: **Question**, **Option**,
**Answer**). It comes from the Document's decisions table or is asked on a Thread. A tap stores the
Answer in Annotary; `pull` writes the head's Answers into the Markdown, `push` refuses a file that
leaves them out, and the server never writes Markdown. Why each choice: `docs/decisions.md`,
2026-11-20.

- **Files:** `crates/renderer/src/questions.rs` (pure, `wasm32`: `find`, `fill_decided`,
  `decided_text`, `check_recorded`, `to_json` / `from_json`; property tests in
  `crates/renderer/tests/questions_props.rs`), `crates/store/src/questions.rs` (`answers`,
  `Store::answer`, `version_questions`, `decided_fills`, the commit rule), migration v8,
  `server/questions.rs` (the routes and `Shown`, which the API, the page and the card read),
  `server/qview.rs` (the page's controls), `ui/questions.js` and `ui/questions.css`,
  `cli/questions.rs` (`answer`, and what `push`, `pull` and `show` share).
- **The decisions table** (`find`): the first table whose header names `Decision`, `Options` and
  `Decided` (case-insensitive, any order, other columns free, `Recommended` optional); a second one
  is a warning. A row is a key (letters, digits, `-`, at most 16, unique, not `t7`-shaped) and 2 to
  6 option ids separated by commas (never `other`); a bad row is skipped with a warning. Parsed with
  pulldown-cmark and `MARKDOWN_OPTIONS` (after the currency pre-pass the renderer runs, so lines agree
  with `data-l`), never with patterns over lines. At most 100 Questions.
- **Places:** the key's first whole-word use outside the table and outside code is the Question's
  place (its block: `p`, `h1`–`h6`, `li`, `td`); its section runs to the next Question's place or a
  heading of the same or a higher level (any heading when the place is not one) or the table. In the
  section, a list item, table row, heading or paragraph whose text starts with an option id (after
  "Recommended:", "Option", "Choice", or followed by `.`, `)`, `:` or a dash) is that Option's place;
  after a lead word the id ends as in `has_word` (`Option us-east` is not `us`). The keys' first
  blocks come from one pass over the words (`named_in`; more than 16 keys), never a scan per key.
  A key the prose never names is a warning; its controls are only in the table.
- **On the Version:** `versions.questions` holds `to_json` (`{key, row, cell, options, rec,
  decided, place, lines, places, digest}`). `push` computes and sends it; the server checks the
  shape and, with `--verify-edge` on, that it equals its own (else 400 `invalid_questions`); a
  commit without it (an older CLI) and a Version from before v8 get the server's own
  (`version_questions`, stored on first read, no event). `GET …/versions/{n}` carries it.
- **Answers** (schema v8, `answers`, append-only): one row per change, a Member's current Answer is
  their latest row for the key (`option` NULL = withdrawn). `digest` is the Question's on the head
  answered, for `changed`. `PUT /api/v1/docs/{id}/questions/{key}/answer {option, version}`
  (`comment`): a table key on the head and undecided there (409 `decided`), or a Thread that asks one
  (its id or label); the same Answer again writes nothing; `version` must be the head (409
  `not_head`). Appends `question.answered {key, option, version}` and bumps the activity; it has no
  `wait` word, so `wait`'s default never wakes on it, and it is never a mention.
  `GET …/questions[?version=N]` answers `Shown` JSON (`server/questions.rs` has the shape); on an
  older Version, the Answers as they stood then (rows given while N was the head or earlier), and
  so does an asked Thread's `answers` in `GET …/threads?version=N` (`add_answers`' `upto`).
- **Across Versions:** Answers follow the key. One whose digest differs from the viewed Version's
  Question is `changed` (`*` in `show`). A filled `Decided` cell makes the Question decided: no new
  Answers, no controls, the cell the truth.
- **A Question's Thread:** `POST …/threads {subject: span, question: KEY}` without a range anchors
  at the place's whole lines (its row when the prose never names it); one per key (409 `conflict`
  naming it). `threads.question` / `threads.options` (labels); Thread JSON carries `question`,
  `options [{id, label}]` and, for an asked one, `answers`. `thread.created` carries `question` /
  `options`.
- **Recording** (`check_recorded`, shared by `push` and `Store::push_version`): for each head
  Question undecided there with Answers, the new file's row with the same key and Options must have
  a non-empty `Decided` cell; a removed row or changed Options passes with a note. Refused: exit 5 /
  409 `answers_missing` with `missing [{key, row, text}]` (the row as it should read). `discard_answers`
  commits anyway and `version.pushed` records `discarded_answers`. `pull` of the head fills with
  `fill_decided` (only empty cells; a short row gains its cells; `|` escaped; a short row is closed
  only when the parser's last cell ends before the line, since GFM reads a trailing `\|` as a
  cell's pipe after any number of backslashes), notes it on stderr; `check_recorded` fills every
  missing row in one call;
  the filled file differs from the Sidecar's base, so `push` sends it, and the next `pull` treats the
  same filled bytes as no local edit (any other difference still refuses).
- **The page** (`qview.rs`, spliced before the Thread strips, so the first paint is final): a
  `.cd-qrow` after the place block, `input.cd-qradio` at an Option block's leading edge, `.cd-qcell`
  in the table's `Decided` cell, `#cd-qsum` / `#cd-qaccept` in the header, and `.cd-qans` leading a
  Question Thread's card (also in the fragment). Everything added carries `cd-qx`, which `app.js`
  keeps out of selection mapping like a strip. Read-only (older Version, deleted, no `comment`):
  buttons disabled. `#cd-data.questions` is the `Shown` JSON, present only with Questions.
  `questions.js` waits for `window.cd` (app.js boots after a frame), answers through `cd.api` (so
  embed mode works), updates optimistically with rollback and a toast, re-reads on
  `question.answered` (`window.cdq`, called by `onLiveEvent`), and opens the Question's Thread for
  **Other…** / **Comment** (`cd.openComposer({kind: 'question', key, line, done})`). An Answer sends
  `D.version` (the Version shown), so a page left behind by a push gets 409 `not_head`; on
  `version.pushed` past it (or that 409) the controls are disabled and **Accept recommended** hidden.
  `boot` waits for the `window.cd` whose `app` is this page's `.cd-app` and stops when its embed
  generation is stale; `app.js`' `main` clears `window.cdq`; `thread.created` with `options`
  re-reads, so a Question asked live answers from its card. The styles and
  script are inlined only on a page that shows a Question.
- **CLI:** `answer DOC KEY OPTION|none` (`KEY  OPTION`; `--json key option changed version`, `doc`,
  `name`); `show`'s `questions: D4=B D5=- D7=A* t9=other` line (omitted when none; two Members
  `D4=B@rowan,A@sam`; `decided`) and `--json questions`; `thread new --ask LABEL` (2 to 6);
  a 409 `answers_missing` from the server (an Answer given after `push` checked) prints the same
  `line N: …` rows (`client::map_status`, `missing[]` under `--json`);
  `thread show` prints `question: D4` or `asks: A "…" B "…" · B@rowan`; `push` warnings, `--strict`,
  the refusal and `--discard-answers`; `pull` fills, `--as-pushed`.
- **Sidecar, purge, export:** the Sidecar's Threads carry `question` / `options`, and it carries
  `answers [{key, by, option, version, at}]`; purge deletes the rows; `export` is the SQLite backup.
- **Tests:** `questions.rs` unit tests and `questions_props.rs` (S50-01, S50-02, S50-05, S50-06),
  `tests/questions_api.rs`, `tests/cli_questions.rs`, `tests/questions_web.rs`,
  `tests/e2e/89-questions/`, `tests/web/questions.spec.ts`, the Question test in
  `tests/web/embed.spec.ts` (S50-13), `tests/cli_skill.rs` (S50-14), the access rows; the live
  scenario `verify/scenarios/questions.spec.ts`.
