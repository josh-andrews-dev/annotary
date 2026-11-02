# Plan: Say "Member" where we mean any Member, and use American English

Status: draft for Josh's review, 2026-11-02. Nothing built.

## What Josh asked for

> On the Members page, there's a button that says "Add agent," but that should say "Add member"
> right? We don't distinguish between agents and people and this button is for any member, right?

> Assuming you agree, scour the UI (web, CLI) and docs for anything else that gets this wrong too.

> And, while you're at it, look for any British spellings or expressions.

## What was checked (2026-11-02)

**Yes, the button should say "Add member".** The model has no person/agent distinction:

- `POST /api/v1/members` takes only `handle` and `name` (`crates/annotary/src/server/members.rs:51`).
- The schema has no person/agent column, and `docs/decisions.md` records rejecting one.
- `docs/CONTEXT.md` says "Nothing in the model distinguishes people from agents".

**One conflict with the premise: a person added this way cannot sign in to a browser today.** The
button makes any Member, but only a Member who runs the CLI can use what it gives them. Checked on a
throwaway server (the installed 0.3.1) and in the code at `ed8495d`:

| Route to a browser session | Result |
|---|---|
| The new Member sets a password over the API | 403: `PUT /api/v1/settings/password` is owner-only |
| An owner sets it for them | Not possible: the route always sets the caller's own password and ignored a `member` field in the body |
| The web pages | No password form for another Member, and none for a role `member` |
| A Token in a browser | Not possible: a browser has no way to send one, and a request with no cookie from a remote client gets 401 |
| `annotary pair` | For the iPhone and iPad app, which has not shipped |
| `annotary password --set --member HANDLE` | The only route (from its help text; not yet run): on the server's machine, with the server stopped |

The login form also has no handle field: `check_password` returns the first Member whose password
matches (`crates/store/src/access.rs:347`), so two people sharing a password sign in as the same one.
`docs/phase1/spec.md` lists "human collaborators/sharing" as backlog, so this gap is known in outline
but nothing tells the owner about it. That is D1.

**Web and CLI text that gets Member wrong.** Proposed wording is under M2.

| # | Where | Says | Problem |
|---|---|---|---|
| W1 | `server/admin.rs:130` | Button "Add agent" | What you asked about |
| W2 | `server/ui/admin.js:117,121` | "Paste it into the agent's environment" … "A person can use it too" | Makes a person the exception. The same panel shows for any minted Token |
| W3 | `server/admin.rs:137` | "the Member, its Membership" | "its" reads as a thing; the glossary says "their" |
| W4 | `server/web.rs:152` | Login page: "Agents and the CLI use a Token" | People use the CLI too |
| W5 | `server/settings_page.rs:40` | Settings search words "Tokens agents Grants…" | Finds the page for "agents" but not "people" |
| C1 | `cli/help.rs:195`, `cli/member.rs:21` | "This is how you add an agent." | Same as W1 |
| C2 | `cli/member.rs:194`, `cli/token.rs:184` | `# put this in the agent's environment` | Also prints when you mint your own Token, and in the human TTY golden |
| C3 | `cli/login.rs:56` | "`annotary member add HANDLE` for an agent" | Same as W1 |
| C4 | `cli/serve.rs:278` | First-run: "mints a token for an agent." | Same as W1, and it is the first thing a new owner reads |
| C5 | `server/error.rs:36` | "owners: `annotary login`; agents: ask an owner" | The worst one: treats owner and agent as opposites, which the glossary's Owner entry warns against, and any Member with a password can log in |

**Docs that get it wrong.**

| # | Where | Problem |
|---|---|---|
| D-a | `docs/annotary-guide.html:645` | "One command per agent", "Add agent" button |
| D-b | `docs/annotary-guide.html:744` | "give an agent its own identity": the glossary avoids "identity" for a Member |
| D-c | `README.md:150` | "how the CLI and agents authenticate": agents use the CLI, and so do people |
| D-d | `docs/phase1/spec.md:51,52,57` | Stories 15, 16, 21: "adding an agent", "Add agent", "agent credentials" |
| D-e | `tests/SPEC.md:273,542` | S12-02 and the story index: "Add agent" |
| D-f | `docs/dev/access-model.md:16`, `docs/dev/web-owner-screens.md:7,14`, `tests/e2e/README.md:155` | "Add agent", `#cd-addagent` |
| D-g | Comments and test titles | `server/admin.rs:5`, `server/ui/admin.js:314`, `server/settings_page.rs:109`, `tests/web/owner.spec.ts:1,33,54`, `crates/annotary/tests/web_owner.rs:5` |

**Checked and correct, so left alone.** These are about agents on purpose:

- The pitch, "humans and agents" (`cli/help.rs:19`, `cli/mod.rs:63,68`, `README.md:3-7`).
- Advice addressed to agents (`cli/help.rs:27,121,155,230`).
- "A person opens" for `annotary link`.
- `crates/annotary/skill/SKILL.md`, which is written to an agent, so "the human" is right.
- `README.md` step 6, which is a walkthrough of adding an agent.
- `docs/phase1/spec.md:24` and the "As a human" web stories.
- The placeholder `scribe`, which is the glossary's own example and pinned by `verify/scenarios/example-text.spec.ts`.
- The "Member" glossary line itself.

**British spellings and expressions.** About 150 lines in 73 files, not counting expressions:

| Where | Lines | Examples |
|---|---|---|
| CLI, shipped | 1 | `cli/mod.rs:96`, the `--no-color` help: "No colour, even on a terminal" |
| Web pages, shipped | 0 | Only code comments |
| Public docs | 6 + 2 expressions | `README.md:88` "straight away", `CONTRIBUTING.md:55` "behaviour", `docs/CONTEXT.md:80` "coloured", `docs/annotary-guide.html:304,397,605` "authorises", "behaviour", and `:864` "at once and it carries on" |
| Spec | 14 | `docs/phase1/spec.md`, `tests/SPEC.md` |
| Dev docs, perf docs, decisions | 33 | "optimisation", "normalises", "cancelled", `DESIGN.md:183` "at once" |
| Source comments and private identifiers | 65 | `normalise_grants`, `normalise_rights`, "centred", "learnt", "needn't" |
| Tests | 31 | Comments, test names, `tests/web/corpus.spec.ts:113` "maths", `tests/e2e/99-ios/ios.trycmd:14` "Have a look" |

None of it reaches the API, the schema or a stored event: no string literal in `crates/` holds one
of these words, and no migration does. So a rename breaks no client.

**Where this has to pass.** The change touches web pages and CLI output, so it needs:

- `make lint test` and `make e2e`, with regenerated goldens.
- A `make verify-live` scenario and a screenshot in the pull request.
- A review through `review-gate`.
- `make deploy` and `make server-health`.

Open issue #11 means `make verify-live` needs `PLAYWRIGHT_BROWSERS_PATH=~/Library/Caches/ms-playwright`
on a Mac. Use that workaround here.

## Decisions

**D1. The person who cannot sign in to a browser.** Renaming the button invites an owner to add a
person, who then gets a Token and no way into the web UI unless the owner stops the server and runs
`annotary password --set --member`. Fixing that properly means deciding how a second person gets a
password and how login tells them apart, because login has no handle. That touches authentication and
is a feature rather than a wording change.
*Decided 2026-11-02 (Josh): "Rename. Open a GitHub issue so that we support adding more people via
the web UI. Don't update documentation otherwise right now."*

**D2. How far the American English sweep goes.** The public text is 8 lines; the rest is for people
reading the code. A full sweep adds one large, mechanical diff but leaves one spelling everywhere, and
it renames private functions with no effect on behavior.
- **Recommended: A.** Everything we own: docs, spec, comments, test names and private identifiers, as
  its own pull request, because nothing reaches the wire and a partial sweep leaves the mix in place.
- B. All prose and comments, leaving identifiers such as `normalise_grants` alone.
- C. Only what ships or is public: the CLI line and the 8 lines in public docs.

**D3. Keeping it that way.** Nothing stops British spelling from coming back, and most of this repo is
written by agents, which drift. `crates/annotary/tests/no_personal_details.rs` is the pattern: a test
that scans everything git tracks.
- **Recommended: A.** Add "American English" to `AGENTS.md` and a test that refuses a short word list
  (`colour`, `behaviour`, `normalis*` and similar), so `make test` catches it, with `fixtures/corpus/`
  exempt.
- B. The `AGENTS.md` line only. There is no cost, but nothing enforces it.
- C. Neither.

## M1: from Josh

- Answer D2 and D3. Reply by ID, or "all recommended".
- Nothing else needs you: agents can do the review, the deploy and the health check.

## M2: Member wording (one pull request, in `josh-andrews-dev/annotary`)

Web:
- W1: "Add member"; rename the id `cd-addagent` to `cd-addmember` in `server/admin.rs`,
  `server/ui/admin.js`, `tests/web/owner.spec.ts`, `crates/annotary/tests/web_owner.rs`,
  `crates/annotary/tests/web_settings.rs` and `docs/dev/web-owner-screens.md`.
- W2: "Token for @x — shown once. Set it where this Member runs annotary:" and "Or save it as a
  profile: `annotary profile add NAME --token …`. Lost it? Revoke it and mint another."
- W3: "the Member, their Membership".
- W4: "The CLI uses a Token instead (`annotary login` turns this password into one)."
- W5: add `people add member`; keep `agents`, since people will search for it.

CLI:
- C1: "This is how you add anyone: a person or an agent."
- C2: `# set this where the Member runs annotary`
- C3: "`annotary member add HANDLE` for another Member"
- C4: "`annotary member add <handle>` adds a Member and mints their token."
- C5: "with a local password: `annotary login`; otherwise ask an owner for a new token"

Docs:
- D-a: "One command per Member", with the Token line neutral and the button named "Add member".
- D-b: "give an agent its own Member".
- D-c: "how the CLI authenticates, for people and agents alike".
- D-d to D-g: replace the wording. Spec ids stay the same.

Per D1:
- File a GitHub issue with `gh issue`: support adding more people through the web UI, including how a
  new person gets a browser password and the login-without-a-handle problem.
- No other documentation change for this gap: no README section on how to add a person. The doc wording
  fixes D-a to D-g stay, because they are the rename.

Tests:
- A bug fix needs a test that fails without it. Assert the button text, and that "Add agent" is gone,
  in `crates/annotary/tests/web_owner.rs`.
- Assert in `tests/web/owner.spec.ts` that the Token panel and member list contain no `/\bagent\b/i`,
  next to the existing `bot` check. The new handle there must change from `agent-…` for this to hold.
- Regenerate goldens with `make goldens` and read the diff. Expected files:
  `10-readonly/help.trycmd`, `10-readonly/env-token-rejected.toml`, `20-mutating/member-add-tty.toml`,
  `80-access-mut/token-mint-tty.toml`, `96-human-tty/human.trycmd`, `00-setup/first-run*.{toml,trycmd}`,
  `75-access/login-no-password.toml`.
- Add `verify/scenarios/add-member-wording.spec.ts`: the button on the real Members screen, and
  `annotary member --help`.

Done when: `make lint test` and `make e2e` pass, `ci` is green, the review is recorded,
`Verified live: <verdict JSON>` and a screenshot of the button are in the pull request, it is merged,
`make deploy` has run and `make server-health` passes.

## M3: American English sweep (a second pull request, from the new `origin/main`)

- Replace per D2, using the map under *Choices I made*.
- `tests/e2e/95-agent-flow/agent.trycmd:13` holds "recognise" in a comment. AGENTS.md makes any change
  to that file a deliberate decision, so record it in the pull request.
- `crates/annotary/skill/SKILL.md` has none, so `make token-report` should change only its header word.
- If D3 is A: add the `AGENTS.md` line and `crates/annotary/tests/american_english.rs`.
- Also fixes `cli/mod.rs:96`, so it needs the same verify-live step, and one more check in the scenario:
  `annotary --help` says "color".

Done when: `make lint test` and `make e2e` pass, `ci` is green, the review is recorded (it passes 200
lines), `Verified live:` is in the pull request, it is merged, `make deploy` has run and
`make server-health` passes. A last search for the word list returns only the exceptions below.

## Stop rules

- Stop and report if a golden changes anything other than the wording above.
- Stop if `make verify-live` fails for a reason the #11 workaround does not cover, or `make deploy` or
  `make server-health` fails.
- Keep going through waits for CI and review.

## Not doing

- **No change to the model, API or schema.** The model is already right.
- **`fixtures/`:** the corpus is realistic test data that anchoring and perf tests depend on, and
  `fixtures/anchoring/TABLE.md` is read by `crates/diff/tests/fixtures.rs`.
- **`docs/archive/`:** history, not live text.
- **`scripts/standard/`, `Makefile`, `.agents/rules/standard.md`:** these come from the template, so
  `scripts/standard/release.py:7` "notarised" has to change upstream.
- **`.github/workflows/release.yml`:** generated by dist.
- **`aria-labelledby`:** an HTML attribute.
- **"shall" in `CONTRIBUTING.md:83`:** Apache license wording.
- **`SKILL.md` "identity" and `docs/annotary-guide.html:1287`:** the skill is at its line limit, and the
  guide row compares products on agent support.

## Risks and cost

- **Money:** none.
- **Effort:** one run, two pull requests.
- **Agents parsing the wording:** C1 to C5 change text that scripts could read, but they are stderr hints
  or TTY-only lines. The non-TTY `ANNOTARY_TOKEN=…` line and every exit code stay the same.
- **Merge conflicts:** the sweep touches many files. Doing M2 first and M3 right after keeps it short.

Monitoring changes: none.

## Choices I made

- Two pull requests, so the wording change can be read without a large mechanical diff around it.
- The exact wording under M2. Change any line by its number.
- Kept `agents` in the Settings search words (W5).
- Left `README.md` step 6 and the guide's agent walkthrough about agents: they show an agent working,
  which is the product.
- Spelling map: -ise to -ize, -our to -or, "centred" to "centered", "cancelled" to "canceled",
  "signalled" to "signaled", "modelled" to "modeled", "learnt" to "learned", "maths" to "math",
  "untick" to "uncheck", "needn't" to "don't need to", "straight away" to "right away",
  "carries on" to "continues", "Have a look" to "Take a look".
- "at once" becomes "immediately" where it means that, in prose and test titles (about 45 lines). It is
  not wrong in American English, only rare; "all at once" stays.
- "afterwards", "backwards" and `RangeError::Backwards` stay: they are common in American English too,
  and the churn buys nothing.
- This plan names no host, since the repo is public.

## Run log

Nothing yet.
