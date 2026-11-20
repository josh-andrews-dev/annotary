# Plan: Pencil markup and structured questions

Status: draft for the maintainer's review, 2026-11-19. Nothing built.

## What the maintainer asked for

Two new ways to give feedback on a Document, planned together in case they share a data model.

- **Markup.** "On the iPad or iPhone, but not on any other surfaces ... mark up the document with
  the Apple Pencil ... freeform. I want the output to go back to the agent, and we leave the agent
  up to interpreting it." At a minimum a PNG. "Scribble notes in the margins, cross out
  paragraphs, rework diagrams." Research how PDF markup tools work and how agents read them.
- **Structured questions.** "Akin to the question-and-answer feature that is popular in
  harnesses": a question, a few options, and "other". Explore (1) a question as a kind of comment
  and (2) "affordances right in the document", plus two more creative options from other model
  families (Gemini Neon and Grok). It must not be "overwhelming and duplicative", and "it
  shouldn't be a one-and-done. It should still have that thread feel."
- The Hermes plugin must be able to use both features in full.
- Coding is done by Opus 5.5 subagents at medium effort; this thread plans and orchestrates.
- The two features may be built independently.

## What was checked (2026-11-19)

**Conflicts first.**

- **The phase 1 spec excludes this.** `docs/phase1/spec.md` lists "comment attachments/embeds" as
  "deliberately not built and not backlogged". A Markup is an image on a Comment, so this plan
  overturns that line, and M2 records it in `docs/decisions.md`. The same list rules out
  "commenting on regions within an image". This plan keeps that: a drawing over a diagram anchors
  to the source lines that hold the image, never to a region of it.
- **"Only on iPad and iPhone" against "hit every surface".** Drawing is only in the app. A Markup
  must still be *seen* wherever its Thread is shown (web, embed mode, the Hermes page) and read by
  agents through the CLI, or the reply to it would have no subject there.
- **The agent skill has no room.** `crates/annotary/skill/SKILL.md` is 60 of 60 lines and 1,086
  tokens against the prototype's 905, which is +20.0% on a ±20% limit (`tests/token-report.md`).
  Each feature gets one sentence that replaces existing words; neither adds a verb to the command
  table.
- **A push stops commenting mid-review.** A question written in the Markdown can only be added by
  pushing a Version, which the skill forbids during a review. A design that only reads the
  Markdown cannot ask a follow-up question.

**The server (this repo, `origin/main` at `e36356d`, release `v0.7.2`).**

- Threads: `threads`, `comments` and `thread_events` tables; subject `span`, `document` or
  `asset`; a Comment body cannot be blank (`docs/dev/threads.md`).
- Comment bodies render without embeds: "images/files become plain links"
  (`docs/dev/rendering.md`).
- The only binary uploads are Version blobs (`PUT /api/v1/blobs/{hash}`, which needs the `write`
  Right) and avatars (base64 in JSON, `PUT /api/v1/me/avatar`). A commenter with only `comment`
  cannot upload a blob today.
- The renderer's sanitizer removes forms, `class` and `data-*` from raw HTML, and task-list items
  render as disabled checkboxes. An agent cannot put a working control in Markdown; the renderer
  itself must mark the place, and each viewer draws its own control.
- YAML front matter is parsed and never rendered. HTML comments are invisible on GitHub.
- Actions: only the last Action on the head Version counts, and a new Version starts with none
  (`docs/dev/actions.md`).
- `wait` wakes on `version,action,mention` by default. Hermes wakes its editor only on Send back
  or Approve, never on a plain Comment.

**The iPhone and iPad app (its `origin/main` at `7f41ca1`).**

- The Document body is the server's HTML in a `WKWebView`, with the app's own `document.css` and
  `document.js`. The native header and margin column are views inside the web view's scroll view,
  so they scroll with the text. A drawing canvas can be added the same way.
- The page already reports each block's source line and top (`lines` in the `layout` message),
  which is what maps a stroke to source lines.
- Text width and font size differ by device, rotation, text-size setting and from the web. Ink
  kept in screen coordinates would not land on the same words anywhere else.
- No PencilKit or drawing code exists. The only upload is the avatar.
- Its rules need a review board of mocks before a feature that neither the web nor its
  requirements describe, and an API change lands in this repo first.
- Minimum iOS 26; no third-party dependencies; UI tests run on leased simulators on a Mac.

**The Hermes plugin (its `origin/main` at `a9a7152`).**

- It pins annotary `v0.7.0`; the latest release is `v0.7.2`.
- Ten `annotary_*` tools call the CLI. `annotary_threads` keeps only named Thread fields, so new
  fields are dropped until it is changed. No tool returns an image.
- Hermes accepts an image from a plugin tool as a `_multimodal` envelope with a `text_summary`
  fallback (`tools/registry.py` in `NousResearch/hermes-agent`); its built-in `vision_analyze`
  also takes a local file path.
- The Document view is this repo's page in embed mode, so page controls arrive with a new pin.
- Size ceilings: tool schemas 5,242 of 5,800 bytes, the skill 4,209 of 4,700. A new tool fits; two
  may not.

**How PDF markup works, and what agents can read.**

- PDF tools keep ink either as vector Ink annotations (`/InkList`, lists of x/y points) or
  flattened into the page. Extraction libraries get coordinates from ink, never text.
- The Claude, OpenAI and Gemini PDF inputs give the model page images plus extracted text. None
  documents whether annotations are drawn in those images. So: send images we render ourselves,
  never a PDF.
- A tall screenshot is unreadable. Claude resizes an image to 1,568 px on its long edge (2,576 px
  on its newest models), so a 820 by 6,000 point page shrinks to about 350 px wide. Crops must be
  cut near each mark.
- Vision models read printed text well and handwriting less well (at most 72% on a 2026
  wild-handwriting benchmark, with "plausible but visually unsupported corrections"). Locating
  which text a mark touches is their weak spot. Giving the model the exact source text beside the
  image is the known remedy (tldraw: "sending both to the model works best").
- `PKDrawing`'s saved bytes are undocumented and change with iOS versions; nothing renders them
  off an Apple device. Stroke points are public API, so strokes can be saved in our own format.
- **Not confirmed:** how well current models read real markup on our Documents. M4 tests it with
  real drawings before the app work is called done.

**The four designs for structured questions.** Two are the maintainer's; Gemini 4 Neon and Grok
4.7 (each at its highest thinking level) critiqued both and proposed more. What follows
summarizes their reports.

*Option 1, a question Thread (the maintainer's).* The agent runs `thread new … --ask A --ask B`.
A margin card shows the choices; Replies follow.

```text
**D4. Where the service runs.**          | t4  D4?
- Recommended: A. The existing box …     |  [ A ] [ B ] [ Other ]
- B. A new VM …                          |  Reply…
```

Works mid-review, and either a person or an agent can ask. Both outside models found the same
faults: the options live twice (prose and card) and drift; ten decisions are ten open Threads
before anyone speaks, which breaks "approve, no Threads open, is agreement"; and the question is
not in the Markdown file, so git has no record of it.

*Option 2, controls in the Document (the maintainer's).* A mark in the Markdown turns a list into
choices. The list items are the options, so nothing is said twice.

```text
**D4. Where the service runs.**
 ( ) Recommended: A. The existing box …
 (•) B. A new VM …                 @rowan
     Other…
```

The faults: by itself it has nowhere to talk, and it cannot ask mid-review without a push.

*Option 3, a ballot on the Action (Gemini).* Front matter lists the decisions
(`D4: {to: "#d4…", opts: [A*, B]}`). A strip under the header shows every decision with an
**Accept recommended** button, and the picks are sent together with Approve or Send back.

```text
Decisions 1/2   D1 [us-east*] [eu-west]   D4 [A*] [B]   [Accept recommended]
```

Best at "D1 A, D2 B, all others recommended, go" in two taps, and the agent wakes once. The
faults: front matter repeats the option names; picks reset with every Version; a strip of bare
letters sends the reader back and forth to the prose.

*Option 4, Needle (Grok).* No syntax at all: a block that starts `D4.` or `Q4.` followed by a
list of `A`, `B` … is a question. Picks are stored under the id `D4`, not under a line range, so
they survive the edit that writes "Decided: B". The first real Comment creates an ordinary
Thread.

The faults: the convention is one author's house style, wrong for a public product, and a silent
miss or a false match cannot be seen while writing. It cannot ask mid-review either.

*Also proposed and set aside:* collapsing the unchosen options so the Document reads as the plan
chosen (both models; hostile while two reviewers disagree, and heavy in the app); ticking any
list item with no question declared (no way to tell a vote from a comment); storing each answer
as a drafted edit to the Markdown (reads as the person editing the Document).

## Decisions

**D1. Which structured-question design to build.** The four designs above each get one thing
right. The recommendation combines them into one model: a **Question** has a stable key, its
**Options**, and one **Answer** per Member; it can come from the Document or from a Thread; its
conversation is an ordinary Thread created by the first Comment.
- **Recommended: E, the combination.** A marked list in the Document becomes the choices (Option
  2), stored under the question's key so answers survive later Versions (Needle). The first
  Comment or "Other" opens a normal Thread on that list. `thread new --ask` asks mid-review
  without a push, in a card (Option 1). The header shows "3 of 5 answered" and **Accept
  recommended**, and the answers arrive with the Action that wakes the agent (the ballot). You
  get in-Document controls with no duplication, the thread feel, and one-tap "all recommended".
- A. Option 1 only. Smallest build; every question is a card that repeats or abbreviates the
  prose, and each is an open Thread.
- B. Option 2 only, with the lazy Thread. No mid-review questions.
- C. Option 3, the ballot strip. Fastest to answer; options named twice; resets each Version.
- D. Option 4, Needle. Your current plans light up with no change; fragile for anyone else.

**D2. How the agent marks a question in Markdown** (applies to E, B and D). The mark must be
invisible or harmless on GitHub and in an editor.
- **Recommended: A, an HTML comment before the list:** `<!-- ask D4 -->`, or
  `<!-- ask D4 rec=1 -->` to name the recommended item. Explicit, invisible on GitHub, fails
  closed, and works for any author's style. The plan-review skill's template gains the line.
- B. Infer it from the `D4.` / `Q4.` shape (Needle). Nothing to write, but it only fits documents
  written in that style, and a near miss fails silently.
- C. YAML front matter (`decisions:`). One place lists every question, at the cost of naming the
  options a second time, far from the prose.

**D3. What one Markup becomes.** You draw in several places, then send. Either each group of
strokes is its own Thread, or the whole session is one.
- **Recommended: A, one Thread per group of strokes,** anchored to the source lines it covers.
  The crossed-out paragraph and the margin note become two cards the agent answers and resolves
  one by one ("Done in v5: cut the paragraph"), they follow their text across Versions, and each
  image is a small crop the model can read. Before sending you see the groups outlined and can
  merge two.
- B. One Document-level Thread per session, holding every page image. Simpler to build; the agent
  gets one long reply to write, nothing re-anchors, and the images are the tall kind models read
  badly.

## M1: from the maintainer

- Answer D1, D2 and D3.
- Approve one review board of mocks (the app's rules require it; the `ux-mocks` flow). It is
  drawn after D1 to D3 are answered and shows: the markup mode on iPad and iPhone (Pencil draws at
  once on iPad while a finger scrolls; a Markup button lets a finger draw; the grouped preview
  before sending), a Markup in a Thread card on the app and the web, and the question controls on
  the web (wide and narrow), in the app and in the Hermes page. About 10 minutes of your time.

Everything after M1 runs without the maintainer.

## The design (as recommended; a different answer to D1 to D3 changes the marked parts)

### Glossary additions (`docs/CONTEXT.md`)

- **Markup**: a drawing a Member makes over the rendered Document in the app, kept on a Comment as
  an image of the marked region and the strokes. *Avoid:* attachment, annotation, sketch.
- **Question**: a choice put to the Document's readers, known by a key that stays the same across
  Versions (`D4`). It comes from a marked list in the Markdown or from a Thread.
- **Option**: one choice of a Question. For a Question in the Document it is a list item.
- **Answer**: one Member's current choice for a Question: an Option, or none. Changing it replaces
  it; the log keeps the history.

### What the two features share

They do not share tables. They share every place a Thread is projected, so they are built in one
order to avoid two runs editing the same lines:

- one appended migration each in `crates/store/src/migrations.rs`;
- `thread_json` / `comment_json`, the Sidecar's `threads[]`, the events log, `export`, purge and
  the blob GC (`docs::referenced_hashes`);
- the one card renderer (`server/ui.rs`) and `app.js`;
- the CLI's `show` / `thread show` rows, the skill, the goldens and the token report;
- the app's `ThreadView` model and `ThreadConversation`; the plugin's `read_threads` and digest.

### Markup

- **Drawing (app only).** A `PKCanvasView` inside the Document's scroll view, above the text and
  below the margin column. iPad: `.pencilOnly`, so the Pencil draws with no mode and a finger
  scrolls and selects as today. A **Markup** toolbar button switches to `.anyInput` for finger
  drawing (the only way on iPhone). Head Version and online only, like a new Thread.
- **Grouping.** Strokes whose boxes touch or sit within a small gap form a group. Each group maps
  to the source lines of the blocks it overlaps (a margin note maps to the block beside it), using
  the page's `lines` report.
- **Sending.** For each group the app renders two files: a PNG of that region of the page with
  the ink on it, in the light theme, at most 1,568 px on its long edge, with about one block of
  context above and below; and the strokes as JSON (points, width, color, time, in the crop's
  coordinates). It then creates a span Thread on the group's lines with an optional typed note.
- **Server.** `POST /api/v1/docs/{id}/markups` (the `comment` Right) takes the PNG and the stroke
  JSON, checks them (a real PNG by decoding, at most 2 MiB and 4,096 px a side; strokes at most
  512 KiB) and stores both as blobs. `POST …/threads` and `…/replies` accept `markup: ID`; a
  Comment with a Markup may have an empty body. Comment JSON gains
  `markup: {image, width, height, strokes}`. Bytes are served at
  `GET /api/v1/docs/{id}/markups/{hash}` (the `read` Right, `nosniff`, immutable). New table
  `comment_markups`; the event is the existing `thread.created` / `thread.replied` with
  `markup: true` in its payload. A deleted Comment drops its Markup.
- **Seeing it.** The card (web wide and narrow, embed mode, the app) shows the image above the
  Comment body; a tap opens it full size. The text lines are highlighted like any span Thread. The
  ink is not redrawn over the live text anywhere, because the text reflows.
- **The agent.** `show` and `thread list` mark the row `[markup]`. `thread show DOC t7 --markup
  DIR` writes `t7.png` (and `t7.1.png` for a Reply's) and prints the paths; the agent opens the
  image with its own image reader. The Thread's quote is the exact source text under the ink, so
  the model gets the image and the text it touches together. Skill: one clause, "a `[markup]`
  Thread is a drawing: `thread show --markup DIR`, then look at the PNG."
- **Hermes.** `annotary_threads` passes `markup` through; the wake-up digest says which Threads
  hold a drawing; a new tool `annotary_markup(doc, thread)` returns the image as Hermes's
  `_multimodal` envelope, with the saved file's path in `text_summary` for models that take a
  path. The embed page shows the image through the new pin.

### Questions (design E)

- **Asking in the Document.** `<!-- ask KEY -->` or `<!-- ask KEY rec=N -->` on its own line,
  directly before a list of 2 to 6 items. `KEY` is letters, digits and `-`, at most 16 characters,
  unique in the Document; `N` is the recommended item's number. Anything else (no list, one item,
  a repeated key) is not a Question, and `push` prints a warning that `--strict` refuses. The
  renderer puts `data-q="KEY"` and `data-rec="N"` on the list (renderer version bump); the Version
  records `questions: [{key, lines, rec, options: [{n, quote}]}]`.
- **Asking mid-review.** `thread new DOC RANGE|--document -m "Question" --ask "Label" --ask
  "Label"` (2 to 6 short labels). The Question's key is the Thread's id (`t7`). Any Member with
  `comment` may ask, through the CLI or the API.
- **Answering.** `PUT /api/v1/docs/{id}/questions/{key}/answer {option: N|null, version}` (the
  `comment` Right; `null` withdraws). One current Answer per Member per key; table `answers` is
  append-only with the Option's quote at the time. Event `question.answered {key, option,
  version}`; it is not in `wait`'s default set and never wakes Hermes. `annotary answer DOC KEY
  N|none` is the CLI form, so an agent can answer a person's Question.
- **Across Versions.** Answers follow the key. Same Option text: the Answer stands. Changed text:
  it stands and is flagged `changed`. Key or item gone: shown under the header as a past Answer
  with its quote, never silently dropped.
- **What a person sees.** Each Option gets a choice mark at its leading edge (a real radio control
  with the item's text as its name; the item's text itself stays selectable for a normal Comment).
  Under the list: who chose what, **Other…** and **Comment**, both of which open the composer for
  that Question's Thread (created on the first Comment, anchored to the list, carrying `question:
  KEY`). The card of that Thread leads with the Answer ("@rowan chose 2: B. A new VM"). A
  mid-review Question shows its labels as buttons in its card. The header shows "3 of 5 answered"
  and **Accept recommended (2)**, which answers every unanswered Question that names a
  recommendation. Actions are never blocked by unanswered Questions. Web: spliced on the server,
  so the first paint is final. App: `document.js` finds `[data-q]` and reports taps over the
  existing bridge; state comes from `GET /api/v1/docs/{id}/questions`.
- **The agent.** `show` gains one line, left out when there are no Questions: `questions: D4=2
  D5=- D7=1* t9=other` (`-` unanswered, `*` changed since answered, `other` a Comment and no
  Option; with two reviewers `D4=2@rowan,1@sam`). `--json questions` gives quotes and Thread ids.
  `wait`'s Action line is unchanged; the agent reads `show` when it comes back, as today. Skill:
  one sentence, "`<!-- ask D4 -->` before a list makes it tappable; `show` prints the answers;
  an answer is not a mention."
- **Hermes.** No new tool for Document Questions (the Markdown mark is enough); `annotary_comment`
  gains an optional `options` list for mid-review asks; `annotary_threads` and the wake-up digest
  carry the `questions:` line; the embed page gets the controls with the new pin. Answers do not
  wake a Bot; Send back and Approve do, as now.

## M2: server, Markup (this repo)

One Opus 5.5 child at medium effort in its own worktree. Read `docs/dev/threads.md`,
`assets.md`, `web-commenting.md`, `web-document-view.md`, `server-conventions.md`,
`cli-conventions.md`. Build the Markup section above: migration, store API through `Store::tx` +
`append_event`, the two routes with rows in `tests/access_api.rs`, GC / export / purge / Sidecar,
the card renderer and embed mode (image under `IMAGE_MAX`), `thread show --markup`, the skill
clause within the limits, goldens, token report, spec ids in `tests/SPEC.md`, `docs/dev/` notes,
`docs/CONTEXT.md`, `docs/decisions.md` (the spec line overturned), the guide.

Done when: `make check` is green; the review named by the review gate has passed; `make
verify-live` passes on the branch with a new scenario that posts a Markup through the API and
sees it in the card; merged and deployed, `make server-health` ok.

## M3: server, Questions (this repo), then release

Same shape, after M2 merges (they edit the same files). Build the Questions section: the renderer
mark and version bump (snapshots, `make wasm-check`), `questions` on the Version, the `answers`
table, routes, the splice and `app.js` controls on every web state in `docs/dev/surfaces.md`
(wide, narrow, dark, an older Version read-only, Compare unaffected, embed mode), `thread new
--ask`, `answer`, the `show` line, the new verb's checklist from `AGENTS.md`, the skill sentence.

Done when: as M2, with a live scenario that pushes a Document with a marked list, answers it and
reads `show`. Then tag the release (`docs/dev/releases.md`); the plugin and the app build against
it.

## M4: the app (its own repo, on a Mac)

Starts when M2 is merged; one Opus 5.5 child at medium effort in a worktree thread on a linked
Mac, Markup first and then Questions (both edit the Document canvas). It follows that repo's
rules: its requirements and design docs updated from the approved board, test-first, every device
pose, fixtures for the new routes, `make lint test ui-test`, light and dark screenshots in the
pull request.

Done when: on a real server, a Pencil drawing made on an iPad simulator build (and a finger
drawing on iPhone) arrives as Threads an agent can read with `thread show --markup`; a marked
list is answered in the app and `show` prints it; the pull request is merged.

The reading test is part of this milestone: five real drawings (a cross-out, a margin note, an
arrow, a reworked diagram, small handwriting) are read by Claude, Gemini and Grok through the
CLI. A reading that is wrong on three of five changes the crop size or adds an ink-only image,
once, and the test runs again.

## M5: the Hermes plugin (its own repo)

Starts when the release exists; one Opus 5.5 child at medium effort. New pin with its checks,
`read_threads` and digest fields, `annotary_markup`, `options` on `annotary_comment`, the skill,
`docs/surfaces.md`, the size report inside its ceilings, and `make live-loop` with a Document
that carries a Question and a Markup.

Done when: in a live loop a Hermes agent asks a Question in a shared Document, reads the Answer
after Send back, and describes a Markup's image; merged.

## M6: close

The plan-review skill's decision template gains `<!-- ask Dn rec=1 -->` (its own repo's pull
request). This plan moves to `docs/archive/`. Every thread this run started is settled.

## Stop rules

The run stops and reports when:

- the skill cannot carry both sentences inside 60 lines and the ±20% token limit (no limit is
  raised);
- a perf budget or size ceiling would have to rise, here or in the plugin;
- the review gate has no reviewer available;
- `make verify-live` fails for a reason the change did not cause;
- the model reading test fails after one change to the image;
- the app needs something the board did not show.

It carries on through waits for CI, review and simulator leases.

## Not doing

- Drawing on the web or in the Hermes page: asked for the app only.
- Redrawing ink over the live text on any device: the text reflows, so it would land on the wrong
  words. The image is the record.
- Handwriting recognition on the device: "we leave the agent up to interpreting it."
- A PDF of the marked-up Document: no provider documents that its PDF input draws annotations.
- General image attachments from the web composer or `thread new --attach`: the route would allow
  it later; nothing asked for it.
- Multiple-choice Questions, Questions that depend on other Answers, blocking an Action on
  unanswered Questions, a web form for a person to compose a mid-review Question (the CLI and the
  API can), answer buttons inside the Hermes chat card (it keeps its Open button), collapsing the
  unchosen Options.

## Risks and cost

- Money: none. No new service, host or account.
- Effort: four coding runs (two here in sequence, the app, the plugin), each with a review. The
  app's is the largest: a new canvas, gestures that must not fight scrolling and selection, and
  every device pose.
- Handwriting is the weak point of today's models; small margin notes may be misread. The source
  quote beside each image and the reading test in M4 are the guard.
- The skill's token limit may force wording an agent follows less well. `--help` carries the
  detail.
- Stored stroke JSON is our own format; nothing reads it yet. It is kept so ink can be redrawn or
  re-cropped later without the original device.

## Choices I made

- Both features in one run, server work in sequence and the app and the plugin in parallel;
  rejected two independent runs because both edit the same Thread projection files.
- The crop is composited over the light theme with a block of context; rejected a transparent
  ink-only PNG as the main image because strokes without the text under them cannot be read.
- Strokes are saved as our own JSON; rejected `PKDrawing` bytes because the format is undocumented
  and changes between iOS versions.
- A Markup is a field on a Comment, not a new Thread subject; rejected a new subject because a
  drawing is feedback about a span, and spans already re-anchor.
- A Markup is shown read-only on the web and in the Hermes page; rejected hiding it there because
  the replies to it would have no subject.
- Markup upload gets its own route under `comment`; rejected reusing `PUT /api/v1/blobs/{hash}`
  because it needs `write`.
- Answers follow the Question's key across Versions and are flagged when the Option's text
  changed; rejected resetting them with each Version because a typo fix would discard ten answers.
- An Answer never wakes an agent; it is read after the Action. Rejected waking on each tap because
  it fights the review handoff decision of 2026-11-19.
- An Option's control is a mark at the item's edge; rejected making the whole item tappable
  because it would fight text selection for ordinary Comments.
- Options are numbered by position and reported with the Answer's quote; rejected parsing a
  letter out of each item's text because it guesses at the author's style.
- "Other" is a Comment on the Question's Thread, not a text field on the Answer; that is what
  keeps the thread feel.
- Gemini's ballot contributes the progress count and **Accept recommended**, not draft answers
  submitted with the Action; rejected drafts because an Answer the agent cannot see until the
  Action has no advantage over one it simply does not wake on.

## Run log

- 2026-11-19: research and the two outside design reports done; plan drafted. Waiting on M1.
