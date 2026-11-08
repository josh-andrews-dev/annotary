---
name: annotary
description: Read, reply to and resolve review Threads on Markdown Documents shared via annotary; push new Versions; take Actions (approve, send back); wait for @mentions; send the human a document to review. Use when a task mentions annotary, a Document id like k7q2m9xd, a Thread id like t7, asks you to address review comments, or you wrote Markdown for the human to decide on.
metadata:
  version: 0.0.0
  cli: annotary
---

# annotary

Humans and agents comment on shared Markdown with equal standing. Pass `-p NAME` on **every** command.
- **Your profile** (`annotary profile list`) is your agent Member, never the human's. None? Ask them.
- **Post as the human** only when they ask ("comment this as me"): their profile, that command only.

## Do this

- **Read through the CLI** (`show DOC`); **never open `*.annotary`**. Cite `t7 "quoted text"`, never line numbers.
- **Pull, edit, push.** `pull DOC` writes `NAME.md`; `push NAME.md -m "…"` uploads a Version. Refused (exit 5)?
  Someone pushed first: merge its diff, push again. Never `--discard-head-changes` unasked.
- **After pushing, answer each Thread you acted on**, briefly, like a colleague: what changed, in which Version
  (`thread reply DOC t7 --resolve -m "Done in v5: split the intro"`), or why not ("Kept: the spec fixes it").
  A question gets an answer, no edit. Unclear? Ask there, don't guess. Resolve only what's done; `@handle` mentions.
- **Links:** give a person only the URL `annotary link DOC [t7]` prints, as a plain link, not in code;
  127.0.0.1 only with them on that machine. T3 Code: `preview_open` it too.
- **Guard `act`, `thread new` with `--version N`**: exit 5 if head moved.
- **Wait, don't poll.** `wait DOC` blocks for a new Version, Action or @mention; `--cursor SEQ` (`status`) resumes.
- **`--json`** to parse (`--json id,anchor,body` narrows, `--json ?` lists).

## A document for the human's review

Markdown you wrote for them to decide on (plans, requirements, design notes, research, handoffs; not
PR bodies or tracker items). The repo file stays the source; push a copy:
1. `D=~/.cache/annotary/review; mkdir -p $D; cp docs/plans/x.md $D/REPO:docs:plans:x.md; push` that file (Name
   `REPO:docs:plans:x`: no `/`). Give its link, say "comment, then Send back or tell me when you're done", `wait` it.
2. **Mid-review, never push** (it stops their commenting) **or add `--on thread,reply`.** Review ends at `send-back`,
   `approve` or their word in chat. An @mention: answer that Thread, no push.
3. Review over: read every open Thread, fix the repo file, copy it over, push once, answer each Thread.
4. `approve`, no Threads open, is agreement: `pull DOC --version N --out $D --force`, copy to the repo path, commit.

## Commands

```
show DOC                          header + open Threads
status DOC                        version · action · open · mentions-me · seq
thread list DOC [--all|--status S|--mentions-me|--since 2h|--sort recent|--version N]
thread show DOC t7                full Thread
thread new DOC "L:C-L:C"|--document|--asset REF -m MSG [--version N]
thread reply DOC t7 -m MSG [--resolve]      thread resolve|reopen|edit|delete DOC t7
mentions [--since 2h]             open Threads mentioning you
pull DOC [--version N]            push FILE.md [-m MSG] [--strict]
act DOC approve|send-back [--version N]     actions DOC
wait [DOC] [--on version,action,mention] [--timeout 30m] [--cursor SEQ]
list [NAME] [--mentions-me|--action ID|none|--since 2h]
```

DOC = id, unique Name, or path to a pulled `.md`. Anchor `refined`: text changed (`thread show`);
`orphaned`: span gone, still replyable. Exit: 0 ok · 1 usage · 2 not found · 3 no credential ·
4 missing right · 5 refused · 6 unreachable · 7 timed out.

`annotary --version` ≠ `metadata.version` above → `annotary skill install`, reload. More: `<command> --help`.
