# Voice

How Annotary's product text reads: CLI output and help, the web pages, error messages and
the public docs. Each new or changed string follows this guide, and the copy pass reads it.
The terms come from [`CONTEXT.md`](CONTEXT.md).

## Audience

- Developers and technical leads reviewing what coding agents write: plans, specs, and design notes.
- Coding agents reading CLI output and the skill file; every extra token costs context.
- Self-hosters installing and running the server on their own machines.
- Readers already know Markdown, Git, and a terminal; do not explain them.

## Tone

- Plain, short, and exact, like a well-written man page.
- People and agents have the same standing: address both the same way, never talking down to an agent or up to a person.
- State what happened and what to do next; no apologies, cheer, or exclamation marks.
- Keep empty states and confirmations brief (for example, `Nothing yet.`).
- Write help lines as fragments starting with a verb or noun; write messages as full sentences.

## Terms

- Capitalize the glossary nouns exactly as defined: Document, Version, Vault, Thread, Comment, Action, Member, Tenant, Sidecar, Asset, and Anchor.
- Write `head` in lowercase for the latest Version; write Versions as `v3`, Thread IDs as `t7`, and Comment IDs as `t7.1`.
- Write Member handles with a leading `@`, such as `@rowan`.
- Use the standard verbs: push, pull, reply, resolve, reopen, delete, restore, and purge. Delete is reversible; purge is permanent.

## Formats

- Use American English (en-US) spelling, punctuation, and date and number formats.
- CLI streams: write results to stdout, and notes and errors to stderr; keep machine output tab-separated with no header.
- Errors name the subject and the rule (`DOC must not be empty`). Include the command to run next when there is a fix.
- Show secrets once and label them clearly (`shown once`).
- Join a status to its consequence with a spaced em dash; put secondary details in parentheses.
- Use sentence case for buttons, labels, and headings: `Copy link`, `Keyboard shortcuts`.

## Avoid

- Marketing and minimizing words: `seamless`, `powerful`, `simply`, `just`, `easy`.
- Synonyms for glossary terms: `file` or `page` for Document, `note` for Comment, `revision` for Version.
- `Please`, `Oops`, `Sorry`, exclamation marks, and emoji.
- Blaming the reader (`you entered an invalid...`); state the rule instead.
- Real hostnames, paths, or names in examples; use `example.com`, `example.ts.net`, and `@rowan`.
