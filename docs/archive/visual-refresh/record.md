# Visual refresh of the web UI: record

The design record for the web UI's visual refresh. Boards: [round 1](round-1.html) (directions),
[round 2](round-2.html) (the chosen direction across every screen) and [round 3](round-3.html)
(states and the final pass), in this folder. The decisions that last go to [`DESIGN.md`](../../../DESIGN.md); this file keeps how
they were reached.

## What it is

Josh's brief (2026-11-07, condensed): "Make it look a bit sleeker. I like the minimal UI and want to
keep it. At the same time, make it a little more lively, fun and visually striking, within
minimalist bounds. Work on that for a few rounds and try to make it better." He approved shipping
it once every gate passes. The page also stars in a demo video for the Hermes Agent plugin
(agents commenting on a shared Document, desktop and phone).

Case: an existing design system (`DESIGN.md`, extracted from `app.css`), refreshed inside its
rules. Josh delegated the picks to the design agent, which takes each round's recommendation
and records it below as decided by the agent, not by Josh.

## Data

Real fixture data: `fixtures/corpus/rfc-tile-cache.md` and its history (v1 to v4), seeded on a
local `annotary serve` (2026-11-08) with three people (Rowan Vale, owner; Mei Tanaka; Tomasz
Reed) and three agents (Hermes, Scribe, Linter), Threads, replies, two resolved Threads, a send
back on v1 and an approve on v4. The seed script is scratch and not committed.

## Verified

- No web font: the perf budgets (`perf/BUDGETS.md`: FCP 100 ms, CLS 0) and embed mode (every
  request goes through the embedder's `postMessage` transport, `docs/dev/web-document-view.md`)
  rule out a font request; checked 2026-11-08.
- Pink is the mark's color and "means nothing else in the UI"; the accent stays blue
  (`DESIGN.md`, Brand, decided 2026-11-01).
- People and agents read alike: `MemberBrief` carries only how a Member signs in
  (`server/ui.rs`); there is no Member kind to badge.
- Disabled controls (the off Version arrow) are exempt from WCAG 1.4.3; the board's audit skips
  `[aria-disabled="true"]`.

## Round 1 (2026-11-08): directions

1. Direction: A (Refine), **B (Margin note)**, or C (Brand color). Recommended B.
2. Document typeface: **T2 A (system sans)** or T2 B (system serif). Recommended T2 A.
3. Filled positive Action button: **yes** or no. Recommended yes.

Self-critique after rendering: B's tie reads as the mark and is the only new element; its ink
ring is heavy; in dark, Reply / Resolve pills and Comment separators vanish because `--cd-soft`
equals `--cd-card` there; the phone h1 hyphenates "two-tier" across lines at 2.05em. A is tidy
but is not "lively". C is striking and breaks two decided rules.

**Decided (agent, 2026-11-08):** 1 B, 2 T2 A, 3 yes. Round 2 applies B to every screen and
settles the ring color and the dark pill ground.

## Round 2 (2026-11-08): B on every screen

Applied B to the Document page (both layouts, both appearances), the list and Settings; added
`--cd-raise` (`#f1f2f5` / `#2c2c2e`) for pills and rules inside a card.

1. Selected ring: **R1 A (ink)**, R1 B (highlight, fails WCAG 1.4.11 at 1.4:1 with Yellow), or
   R1 C (accent, reads as focus). Recommended A.
2. Hollow green ring for resolved Threads: **yes**. Recommended yes.

Self-critique: the board placed strips by hand and t6 overlapped t1; the real script pushes
strips apart, which leaves a selected card below its highlight, so the straight tie can miss it.
The Document-level card repeats "whole document" under the strip head (left as is: the card is
the same element in both layouts).

**Decided (agent, 2026-11-08):** 1 A, 2 yes.

## Round 3 (2026-11-08): states

Drew the tie as an elbow for a pushed card, every card state, the header's states, embed on a
phone and sign-in. The audit caught the posting Comment: today's 72% opacity puts its meta at
2.9:1, so a pending Comment is now full opacity with muted text.

1. The tie bends when the card was pushed down: **bend**, or hide. Recommended bend.
2. Drop the 14 px nudge on selection: **yes**. Recommended yes.

**Decided (agent, 2026-11-08):** 1 bend, 2 yes. Build.

## Build notes (2026-11-08)

What the build changed from round 3, each found by a test or by looking at the real page:

- Highlights have no hover transition (round 3's motion table had one): a mid-fade background made
  the resolved toggle's own test read the old color.
- The Action chip is hidden from 850 to 1800 px, where the status line says the same, and the
  gaps tighten to 8 px under 1366 px, so the header holds one row with Compare (which merged during
  the build). Hiding the Vault crumb was tried and reverted: `DESIGN.md` (Brand) keeps it there.
- The tie's dot was dropped: the elbow is drawn with borders, and the card is the note.

## Cut from the boards

- Per-Member name colors and an "AI" badge (drawn in C, rejected): identity comes from the avatars
  another change adds.
- A web font (not drawn): see Verified.
