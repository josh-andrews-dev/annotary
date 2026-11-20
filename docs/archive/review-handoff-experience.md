# Plan: choose the review handoff at setup, and wake the agent with a webhook

Status: Approved 2026-11-19 (all recommended), with two requirements added at approval. Nothing
built yet.

## What the maintainer asked for

- When an agent hands the maintainer a plan and waits for approval, T3 Code shows the thread as Working,
  where their settings hide it. They do not want to change their T3 settings.
- Option 3a: Annotary wakes the agent's thread itself when they press Send back or Approve, or
  @mentions it.
- "I'd prefer that a user and agent decide together at setup time what the right experience is
  for them. Annotary should be flexible."
- At approval: "Make sure this works in T3 and also in Claude Code. Make sure other tools can
  build their own version during setup on top of what T3 provides." So T3 Code is one recipe on
  a generic contract, Claude Code gets its own recipe, and any other tool can build one during
  setup from the contract alone (M5).

## What was checked (2026-11-19)

- **Conflict first:** a T3 webhook trigger is tied to one thread (`schedule_task` with
  `bindToCurrentThread`), but a setup choice is made once. So setup picks the *experience*, and
  the agent does the wiring for each review: it makes a trigger for its own thread, registers it
  with Annotary for that one Document, and removes it afterward.
- **Why the thread shows Working:** the skill says "`wait` it". A blocking `wait`, or a
  background one kept with `t3_background_wait` ("Keep this thread in Working while background
  work runs", from T3's own tool description), keeps the agent's turn open.
- **Option 2 merged** in #82 (`feat(skill): ask the human instead of waiting on a review
  handoff`): the agent ends its turn with a question. This plan builds on top of it and edits
  the same skill section, so read that section on `main` first.
- **No first-time setup exists for agents.** The README's "About five minutes" walkthrough is for
  people. `SKILL.md` (at most 60 lines, "no references file") is the everyday skill. Its only
  setup line is "None? Ask them." about the profile.
- **Somewhere to store the choice already exists.** Member settings (CMD-23,
  `crates/store/src/settings.rs`) support user settings and device settings, with the Device
  kind `cli`. Resolution is `device` > `all` > `default`. The CLI has `settings get|set
  --device NAME|--all-devices`. A setting needs only an entry in `CATALOG` and client behavior.
- **Nothing in Annotary sends outbound HTTP today.** No webhook or notifier exists in the
  docs or the code. `ureq` 3 with rustls is already a dependency of the `annotary` crate (for
  the CLI's client).
- **The events log fits delivery.** `GET /api/v1/events` has a durable `seq` cursor, the filter
  words `version,action,mention`, a rights check per wake (`Filter::refresh`, CMD-8), and
  body-free payloads by decision ("payloads stay body-free"). A deliverer can be a cursor per
  hook over the same log, with no outbox.
- **T3's webhook trigger** (`schedule_task`, `schedule: {type: 'webhook'}`): "only creation
  returns it, with T3 Connect and run permission". It supports an optional signature header
  (`hex|base64`, prefix such as `sha256=`, a stored secret), body filters, dedupe keys and
  `maxDeliveryAgeMinutes`. The tool says to "treat webhook request values as untrusted". **Not
  confirmed:** how the request body reaches the woken turn's prompt. M2 checks that first.
- **Claude Code can wake itself without a webhook.** Its Bash tool's `run_in_background` "runs
  the command detached: it keeps running across turns and re-invokes you when it exits" (the
  harness's tool description). So in Claude Code, *hook* can be a background
  `annotary wait DOC --on action,mention` started just before the turn ends. The session is idle,
  and the event's line is what the agent sees when it is re-invoked. **Not confirmed:** whether a
  T3 thread running Claude Code shows idle or Working while that background command runs
  without `t3_background_wait`. M2 checks it. If it shows idle, T3 also gets this simpler
  recipe as an alternative.
- **annotary-hermes** is a public plugin repo with the same review flow. Hermes has its own way
  to wake, so it gets an issue there, not code in this run.

## Decisions

**D1. Which experiences setup offers, and the default.** *Decided 2026-11-19: Recommended (maintainer).* These are what an agent can do after
handing over a Document: *wait* (today's blocking `wait`; right for a terminal session the human
watches), *ask* (option 2: end the turn with a question in the harness), *end* (end the turn and
say "tell me in chat when you're done"; for harnesses without questions), *hook* (end the turn;
Annotary's webhook wakes the thread on Send back, Approve or an @mention). The default is what
an agent does before anyone has chosen.
- **Recommended: all four, default *ask*.** Every harness has a fit, and the default matches what
  option 2 ships now.
- B. Only *ask* and *hook*, default *ask*. Fewer words in the skill, but a terminal user loses
  *wait*.

**D2. Where the choice lives.** *Decided 2026-11-19: Recommended (maintainer).* The same agent Member can run in T3 on one machine and a plain
terminal on another, and the right experience depends on the harness.
- **Recommended: a device setting `review_handoff`** (Device kind `cli`, so web, iPhone and iPad
  ignore it). Setup saves it for this machine (`--device`) or everywhere (`--all-devices`). This
  reuses CMD-23's resolution and sync, and the choice is on the server, so the owner can see it.
- B. A user setting: one choice per agent Member everywhere. Simpler, but wrong for an agent used
  from two harnesses.
- C. A field in the local profile in `config.toml`. Per machine, but invisible to the server and
  the owner, and outside the settings machinery.

**D3. Who may register a webhook.** *Decided 2026-11-19: Recommended (maintainer).*
- **Recommended: any Member, for itself.** The hook only carries events that Member can already
  read (rights are re-checked at every delivery, as for `wait`). An owner can list and remove
  anyone's hooks.
- B. Owners only. An agent could not set up its own *hook* experience, so the human would do it
  each time, which defeats the flow.

**D4. Which addresses the server may call.** *Decided 2026-11-19: Recommended (maintainer).* A server that calls URLs given by Members can be
pointed at internal services (SSRF).
- **Recommended: `https` only, public addresses only, plus an owner allowlist.** Any host that
  resolves to loopback, private, link-local or tailnet (CGNAT) ranges is refused, unless the
  server's `ANNOTARY_HOOK_ALLOW_HOSTS` setting (in `/etc/annotary/env`) lists it. The address is
  checked again at every delivery (against DNS rebinding). T3 Connect's webhook URL is public, so
  the T3 case needs no allowlist.
- B. Any URL. Simpler, but a Member's Token could probe the server's network.

**D5. How the agent and human make the choice at setup.** *Decided 2026-11-19: Recommended (maintainer).*
- **Recommended: a "First use" line in the skill and a help topic, `annotary help setup`.** The
  skill says: if `settings get review_handoff` reads `default`, nobody has chosen yet, so ask the
  human (the four experiences, one line each, from `help setup`) and save the answer. The
  detail lives in `--help`, which is already the reference, so the skill stays within 60 lines.
  Changing it later is the same conversation ("switch to hook").
- B. An `annotary setup` verb that prompts. Agents handle interactive prompts badly, and it
  duplicates `settings set`.
- C. A choice in the web Settings screen. Agents do not use the web, and the decision should be
  made with the agent, in its harness.

## M1: from the maintainer

- Confirm T3 Connect is on and this harness may create webhook triggers (T3 Settings), since
  `schedule_task` returns a webhook URL only then. About one minute.

Everything after M1 runs on its own, in one run in this repo.

## M2: confirm how T3 Code and Claude Code wake

- **T3 Code, webhook:** in a scratch T3 thread, create a webhook trigger bound to that thread,
  POST a sample Annotary payload with a signature, and record what the woken turn receives (the
  prompt, the body values, the dedupe and signature behavior). Delete the trigger.
- **Claude Code, background wait:** in a plain Claude Code session (terminal, not T3), start
  `annotary wait DOC --on action,mention` with `run_in_background`, end the turn, take an Action
  on a test Document, and record that the session was re-invoked with the event line.
- **T3 Code, background wait:** the same in a T3 thread, without `t3_background_wait`. Record
  whether T3 shows the thread as idle or Working while it runs, and whether the re-invocation
  arrives.
- Write each contract in `docs/dev/hooks.md` (an external contract, so it belongs in `docs/`),
  with no private host or URL in it.

Done when: a real T3 webhook started a turn in its bound thread, a real Claude Code session was
re-invoked by a background `wait`, and the doc records what each turn saw. If the T3 webhook body
cannot reach the turn and the background wait shows as Working in T3, **stop** and report.

## M3: hooks in the store and server

- **Spec first:** a new section in `tests/SPEC.md` with ids for: registration, rights,
  address refusal, delivery, signature, retry and disable, `--once`, expiry, and shutdown.
- **Migration** (appended to `crates/store/src/migrations.rs`): `hooks (id, member_id, doc_id
  NULL, kinds, once, url, secret, cursor_seq, expires_at, created_at, last_status, failures,
  disabled_at)`. The URL holds T3's token, so it is a secret. The API never returns it, only
  its host and a `sha256[:10]` fingerprint. Every mutation goes through `Store::tx` +
  `append_event` (`hook.added|removed|disabled`, payloads with no URL).
- **Routes:** `POST|GET /api/v1/me/hooks`, `DELETE /api/v1/me/hooks/{id}`,
  `POST /api/v1/me/hooks/{id}/test`, and the owner's `GET /api/v1/hooks`. A row each in
  `tests/access_api.rs`'s `endpoints()`. New errors map in `server/error.rs` (refused address
  → 409 → exit 5).
- **Deliverer:** one task woken by the commit hook (`AppState.events`). For each live hook, it
  reads events after its cursor with that Member's `Filter` (refreshed per wake, as CMD-8 does)
  and POSTs one body-free JSON event, versioned for other tools to build on (`v: 1`): `kind`, `doc`, `name`, `version`, `thread`, `action`, `by`
  handle, `seq`, `at`, and the `annotary link` URL. No Comment text, so the woken agent reads
  through the CLI, which also keeps untrusted text out of the prompt. It signs with HMAC-SHA256
  (`X-Annotary-Signature: sha256=<hex>`, matching T3's signature options). It uses `ureq` in
  `spawn_blocking` with a 10 s timeout and checks the address first (D4). It retries with backoff
  for up to 24 h, then disables the hook with a `hook.disabled` event. A `--once` hook is
  removed after its first 2xx response. Hooks expire after 7 days by default. The deliverer
  watches `st.shutdown` and stops within `DRAIN_MAX`.
- Tests: in-process tests with a local receiver (allowlisted in the test), and a failpoint for
  retry.

Done when: `make test` is green with every new spec id covered, and a test proves a Send back
reaches the receiver signed, once, with no body text.

## M4: the CLI

- `annotary hook add [DOC] --on version,action,mention [--once] [--expires 7d]` reads the URL
  **from stdin** (never on the command line, per the secrets rule) and prints the id and secret
  once (the secret is also shown for T3's signature). Also `hook list`, `hook remove ID`, and
  `hook test ID`.
- Each verb follows `docs/dev/cli-conventions.md`: TTY and non-TTY cases, `--json` fields, a
  line in `json-fields.trycmd`, every exit path, an entry in `cli/help.rs::TOP`, and a
  regenerated `help.trycmd`.

Done when: `make e2e` is green with the new goldens read.

## M5: the setting, setup and the skill

- `review_handoff` in `CATALOG`: a device setting, Device kind `cli`, values `wait|ask|end|hook`,
  default `ask` (D1, D2). A second device setting, `review_wake` (free text, default empty),
  names the *hook* recipe: `t3-webhook`, `background-wait`, or a name the tool chose at setup.
- `annotary help setup`: the four experiences and what each looks like in T3 Code, Claude Code,
  a terminal and Hermes, then the *hook* recipes:
  - **`t3-webhook` (T3 Code):** create a webhook trigger bound to this thread with T3's signature
    header, pipe its URL to `hook add DOC --once --on action,mention`, end the turn; when woken,
    remove the trigger and continue at "Review over".
  - **`background-wait` (Claude Code; T3 too if M2 shows it idle):** start
    `wait DOC --on action,mention --timeout 7d` in the background, end the turn; the event line
    that wakes the session says what happened. No server hook needed.
  - **Build your own (any other tool):** what a tool needs (a way to resume the agent on an
    inbound HTTPS POST, or when a background command exits), the `v: 1` payload, how to verify
    the signature, and how to prove it at setup with `hook test`. The agent and the human write
    the recipe together, save its steps where that tool keeps its instructions (its own skill or
    rules file), and save its name in `review_wake`. The T3 recipe is the worked example.
- `SKILL.md`: a "First use" line (D5), and the review step reads `review_handoff` to decide what
  happens after handing over the Document. The skill stays at 60 lines or fewer. This updates the
  goldens and `tests/token-report.md` (`make goldens`, `make token-report`). If
  `95-agent-flow/agent.trycmd` changes, the pull request records why.
- Surfaces (`docs/dev/surfaces.md`): the web and iOS screens ignore a `cli` device setting (a
  test proves it). Update the API docs, the README's agent section, `docs/dev/hooks.md`,
  `docs/dev/README.md`'s index, and `docs/decisions.md` (the choices below). File an issue in
  annotary-hermes with `gh issue` to map `review_handoff` onto Hermes's own wake.

Done when: a fresh agent asked to "use annotary" with no saved choice asks the human, saves the
answer, and follows it, in both T3 Code and Claude Code; and an agent in a third harness (Codex
CLI) builds a working *hook* recipe at setup from `help setup` alone, proved with `hook test`.

## M6: ship and prove it live

- The pull request is a server change of more than 200 lines, so it goes through `review-gate`.
  Then CI, `make deploy` from a clean `origin/main` checkout, `make server-health`, and
  `make verify-live` with a new `verify/scenarios/` scenario on the real data: an agent Member
  sets `review_handoff=hook`, registers a `--once` hook to a T3 webhook bound to a test thread,
  and a Send back on a test Document wakes that thread. The same run proves `background-wait`
  in a plain Claude Code session on the live server.
- "Monitoring changes:" an outbound deliverer and `hook.disabled` events. Add a count of failed
  hooks to `deploy/healthcheck.sh`'s verdict and to the maintainer's monitoring in the same
  change.

Done when: on the live server, pressing Send back in the web page resumed a real T3 thread that
had shown as idle, an Approve resumed an idle Claude Code session, and `Verified live: <verdict>`
is in the pull request.

## Stop rules

- Stop and report if M2 shows T3 Code has no recipe that wakes an idle thread, if Claude Code's
  background `wait` does not re-invoke the session, or if either needs a setting the maintainer
  has not given.
- Stop if the review gate finds a security problem that changes D3 or D4.
- Stop if `make verify-live` fails twice for a reason outside this change.
- The run carries on through waits for CI and review.

## Not doing

- Notifications for people (ntfy, email, Slack). Hooks are generic, so this can come later
  without redesign.
- Hooks for a whole Vault, and a web screen for hooks. The CLI and the owner's list are enough
  for agents.
- Comment text in payloads. This keeps events body-free and the prompt free of untrusted text.

## Risks and cost

- **Cost:** no money. Outbound calls go from the existing server. Effort: one large pull
  request (store, server, CLI, skill, docs), reviewed.
- **SSRF:** the outbound requests are the main security surface; D4 and the review gate cover it.
- **Stale hooks:** an agent that never wakes leaves a hook behind. Expiry after 7 days and
  `--once` limit that.
- **T3 Connect availability:** if the relay is down, delivery retries for 24 h, and the human
  can still tell the agent in chat.

## Choices I made

- Body-free payloads with a link, to match the events log's decision.
- HMAC-SHA256 with the `sha256=` prefix, so T3's signature check works without glue.
- A cursor per hook over the events log, not a separate outbox.
- `ureq` in `spawn_blocking`, reused, not a new HTTP client.
- `--once` plus a 7-day expiry by default.
- *hook* is one experience with several recipes (`review_wake`), not a fifth value for each
  harness, so D1's four values stay and any tool can add a recipe without a release.
- Claude Code's recipe is a background `wait`, not a webhook: it needs no server hook and no
  public URL, and Claude Code has no documented inbound trigger for an idle session.
- The payload carries `v: 1`, so a recipe another tool builds keeps working as fields are added.
- The URL is read from stdin and never echoed.

## Run log

- (empty)
- 2026-11-19, M3 done: migration v7 (`hooks`), `annotary_store::hooks`, `server/hooks.rs` (routes
  and the deliverer), spec ids S49-01…S49-10, `tests/hooks_api.rs`; `make lint test` green.
  Choices: a hook delivers as the Token that added it (revoked ⇒ disabled `credential`); the
  Member's own events never wake it; 404 / 410 disable at once (T3 answers 404 for a deleted
  trigger); delivery bookkeeping is a hand-made transaction with no event, as `touch_device`;
  each POST runs on its own thread so shutdown never waits for it; the D4 check sits inside
  ureq's resolver (`unversioned`, pinned by `Cargo.lock`) so the checked address is the one
  dialed. The `review_handoff` / `review_wake` settings (S49-11, M5) landed here too: `cli_only`
  settings are kept out of the catalog, the matrix, iOS sync and the web search.
- 2026-11-19, M4 done: `annotary hook add|list|remove|test` (`cli/hook.rs`), `annotary help
  [setup]`, `settings … --device this` (this install's own `cli` Device, its id kept in
  `config.toml`'s `device`; without it no agent could save a per-machine choice, since the CLI
  never made a Device), goldens in `tests/e2e/89-hooks/`, `json-fields`, `help`; `make lint test
  e2e` green. Choices: `hook add` takes the receiver's own secret as an optional second stdin
  line, because a T3 trigger needs its signing secret at creation, before Annotary would make
  one; the top-level help and SKILL.md sat at their token ceilings (825 of 826, 1086 of 1086), so
  room came from trimming parentheticals and rarely used flags there, not from raising either.
- 2026-11-19, M5: settings, `help setup`, the skill, docs and the health check done; the
  annotary-hermes issue is josh-andrews-dev/annotary-hermes#61.
  - Claude Code (a plain terminal session): the first skill wording failed. A separate "First use"
    line was skipped and the agent went straight to `wait`. Moving the
    `settings get review_handoff --device this` call into review step 1 fixed it. The next fresh
    agent asked the human (Hook), saved `hook` + `background-wait` with `--device this`, and ended
    its turn with a background `wait`. A Send back re-invoked the idle session: it answered the
    Thread, pushed v2 and re-armed. An Approve re-invoked it again, and it committed.
    The test session ran with its own `CLAUDE_CONFIG_DIR`, because a user-level v0.3.1 annotary
    skill in `~/.claude/skills` shadows the project's copy. That is the machine's setup, not
    this change.
  - T3 Code (a scratch thread, pointed at the new skill file for the same reason): the agent
    asked through T3's question card, saved `hook` + `t3-webhook`, made a webhook trigger bound
    to its thread, and registered a `--once` hook to the relay. The thread was idle (run
    `completed`). The Send back woke it through the webhook, with the templated prompt. It
    deleted the trigger, answered t1 and pushed v2. Finding: it did not arm again for the
    Approve round, so `help setup`'s T3 recipe now says to. It also notes that `hook test` wakes
    the thread at once.
  - **Not done: the Codex CLI check.** Codex is disabled in T3 on this machine and on both
    linked Macs, and no `codex` binary is installed here. It needs the maintainer to enable it;
    it was not swapped for another tool.
- 2026-11-19, M6 (stopped at a stop rule). Review went through `review-gate` with Gemini 4 Neon
  High: three rounds (fix first, merge, merge). The fixes:
  - Pages skip `cli_only` settings, because of the statement budgets.
  - The per-pass failure state and the full-page wait are fixed.
  - `hook test` no longer holds shutdown.
  - A purged Document's id shows in `hook list`.
  - A host outside ONLY_HOSTS is refused before its lookup.

  One open low: no tests for the last two. Planning verify-live found a hazard: a copy of the live
  data would carry the live hooks. `ANNOTARY_HOOK_ONLY_HOSTS` now limits the copy to loopback, and
  `verify/scenarios/review-handoff-hooks.spec.ts` was added.

  `make verify-live` on a Mac at 756523e: 25 of 26 passed, `review-handoff-hooks` included.
  `dollar-amounts` failed twice, and the same way on `main`, so the cause is outside this change
  (#86). The stop rule applies: no merge, no deploy, no live proof yet. Also found: verify-live's
  Playwright path on a Mac (#87). CI's `perf` check needs a `make perf` table from a Mac, because
  of the `prefs.rs` fix.
