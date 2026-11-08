#!/usr/bin/env python3
"""Hold the files each agent harness loads at session start to the standard's budget. Exit 1 on
a finding.

    python3 scripts/standard/always_loaded.py [repo-root]

The set each harness loads:
- Claude Code and Grok: CLAUDE.md (and .claude/CLAUDE.md) with its `@` imports, AGENTS.md for
  Grok, and every Markdown file under .claude/rules (the link to .agents/rules is followed);
- Antigravity: AGENTS.md, GEMINI.md and the .agents/rules/*.md files marked `trigger: always_on`;
- Muse: AGENTS.md.
Each set also carries the global rules' budget (global.md), read from ~/.agents/rules/budget.json
where the machine has one, or else, as on a GitHub-hosted runner, from budget.json beside this
script.

Fails on: a file at the per-file cap or over; a set over the combined token limit (bytes / 4, its bytes_per_token); the
repo's own files, each counted once, over its ratchet (`always_loaded_budget` in
.agents/standard.yml); a file neither the standard nor `always_loaded_extra` names. Warns on a file
over the line warning, and while no ratchet is set. Reads tracked and untracked (not ignored)
files only. Stdlib only; runs on Python 3.9.
"""
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEPLOYED_BUDGET = os.path.join("~", ".agents", "rules", "budget.json")
PINNED_BUDGET = os.path.join(HERE, "budget.json")
# The deployed budget.json's key names; the pinned copy beside this script uses the same ones.
BUDGET_KEYS = ("global_md_bytes", "file_bytes_max", "combined_tokens_max", "bytes_per_token", "lines_warn")
STANDARD_FILES = {"AGENTS.md", "CLAUDE.md", "GEMINI.md", ".agents/rules/standard.md"}
HARNESSES = ("Claude Code", "Grok", "Antigravity", "Muse")
# Claude Code follows `@path` imports up to five hops deep; one inside a code span or block is text.
IMPORT = re.compile(r"(?:^|\s)@((?:\.{0,2}/)?[\w.\-/]+)")
MAX_IMPORT_DEPTH = 5
UPDATE = "uvx copier update --trust --defaults --vcs-ref=:current: --answers-file .agents/standard.yml"


class NotARepo(Exception):
    pass


def files(root):
    result = subprocess.run(
        ["git", "-C", root, "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        capture_output=True)
    if result.returncode != 0:
        raise NotARepo(root)
    return {p for p in result.stdout.decode().split("\0") if p and os.path.isfile(os.path.join(root, p))}


def read(root, path):
    with open(os.path.join(root, path), encoding="utf-8", errors="replace") as f:
        return f.read()


def load_budget():
    """(the budget, where it came from). The deployed file wins; a key it lacks comes from the
    pinned copy, so an older deploy still gives every limit."""
    with open(PINNED_BUDGET) as f:
        budget = json.load(f)
    source = "the pinned scripts/standard/budget.json"
    deployed = os.path.expanduser(DEPLOYED_BUDGET)
    if os.path.isfile(deployed):
        try:
            with open(deployed) as f:
                found = json.load(f)
        except (OSError, ValueError) as e:
            print("always-loaded: %s is unreadable (%s); using the pinned budget" % (deployed, e), file=sys.stderr)
        else:
            budget.update({k: found[k] for k in BUDGET_KEYS if isinstance(found.get(k), int) and found[k] > 0})
            source = DEPLOYED_BUDGET
    return budget, source


def unquote(value):
    """A one-line YAML scalar's text, or None when a quote is not closed on the line (a wrapped
    value this reader cannot follow)."""
    value = value.strip()
    if value[:1] == "'":
        if len(value) < 2 or value[-1] != "'":
            return None
        return value[1:-1].replace("''", "'")
    if value[:1] == '"':
        if len(value) < 2 or value[-1] != '"':
            return None
        try:
            return json.loads(value)
        except ValueError:
            return None
    if value[:1] in ("|", ">"):
        return None
    return re.sub(r"(?:^|\s+)#.*$", "", value)


def answers(root):
    """(always_loaded_budget or None, {path: reason} from always_loaded_extra, problems), from the
    answers file Copier writes (`to_nice_yaml`: a block list of `path:`/`reason:` mappings)."""
    budget, extra, problems = None, {}, []
    redo = "record it again with `%s -d ...`" % UPDATE
    inside, item = False, None
    for line in read(root, ".agents/standard.yml").splitlines():
        m = re.match(r"^always_loaded_budget:(.*)$", line)
        if m:
            value = unquote(m.group(1))
            if value is None or not re.match(r"^\d+$", value):
                problems.append(".agents/standard.yml: always_loaded_budget is %r, not a byte count; %s"
                                % (m.group(1).strip(), redo))
            else:
                budget = int(value) or None
        m = re.match(r"^always_loaded_extra:(.*)$", line)
        if m:
            inside = True
            if unquote(m.group(1)) not in ("", "[]"):
                problems.append(".agents/standard.yml: always_loaded_extra is %r, not the block list Copier "
                                "writes; %s" % (m.group(1).strip(), redo))
            continue
        if inside and line and not line[0].isspace() and not line.startswith("-"):
            inside = False
        if not inside or not line.strip() or line.strip().startswith("#"):
            continue
        m = re.match(r"^\s*(-?)\s*(path|reason):\s*(.*)$", line)
        if not m:
            problems.append(".agents/standard.yml: always_loaded_extra has a line this check cannot read (%s); %s"
                            % (line.strip(), redo))
            continue
        if m.group(1) or item is None:
            item = {}
        value = unquote(m.group(3))
        if value is None:
            problems.append(".agents/standard.yml: an always_loaded_extra %s runs past its line (%s); %s"
                            % (m.group(2), m.group(3).strip(), redo))
            value = ""
        item[m.group(2)] = value
        if item.get("path"):
            extra[item["path"]] = item.get("reason", "")
    return budget, extra, problems


def frontmatter(text):
    if not text.startswith("---\n"):
        return {}
    end = text.find("\n---", 4)
    if end < 0:
        return {}
    out = {}
    for line in text[4:end].splitlines():
        m = re.match(r"^([A-Za-z_]+):\s*(.*?)\s*$", line)
        if m:
            out[m.group(1)] = unquote(m.group(2))
    return out


def imports(root, path, paths, seen, depth=0):
    """`path` and every repo file it imports with `@`, in load order. Imports of files outside the
    repo (`@~/…`) and of files git does not list are skipped: they load nowhere a check can see."""
    if path in seen or depth > MAX_IMPORT_DEPTH:
        return []
    seen.add(path)
    out = [path]
    text = re.sub(r"```.*?```", "", read(root, path), flags=re.S)
    text = re.sub(r"`[^`\n]*`", "", text)
    for m in IMPORT.finditer(text):
        target = m.group(1)
        if target.startswith("~"):
            continue
        rel = os.path.normpath(os.path.join(os.path.dirname(path), target))
        if rel in paths:
            out += imports(root, rel, paths, seen, depth + 1)
    return out


def claude_rules(root, paths):
    """The Markdown under .claude/rules, as repo paths, following the folder link to .agents/rules."""
    base = os.path.join(root, ".claude", "rules")
    if not os.path.isdir(base):
        return []
    real_root = os.path.realpath(root)
    out = []
    for folder, dirs, names in os.walk(base, followlinks=True):
        dirs.sort()
        for name in sorted(names):
            if not name.endswith(".md"):
                continue
            rel = os.path.relpath(os.path.realpath(os.path.join(folder, name)), real_root)
            if rel in paths:
                out.append(rel)
    return out


def harness_sets(root, paths):
    seen = set()
    claude = []
    for start in ("CLAUDE.md", ".claude/CLAUDE.md"):
        if start in paths:
            claude += imports(root, start, paths, seen)
    claude += [p for p in claude_rules(root, paths) if p not in claude]
    agents = ["AGENTS.md"] if "AGENTS.md" in paths else []
    always_on = sorted(p for p in paths if re.match(r"^\.agents/rules/[^/]+\.md$", p)
                       and frontmatter(read(root, p)).get("trigger") == "always_on")
    return {
        "Claude Code": claude,
        "Grok": agents + [p for p in claude if p not in agents],
        "Antigravity": agents + (["GEMINI.md"] if "GEMINI.md" in paths else []) + always_on,
        "Muse": agents,
    }


def size(root, path):
    return os.path.getsize(os.path.join(root, path))


def check(root):
    """(problems, warnings, the repo's own always-loaded bytes, where the budget came from)."""
    paths = files(root)
    budget, source = load_budget()
    answered = ".agents/standard.yml" in paths
    ratchet, extra, problems = answers(root) if answered else (None, {}, [])
    sets = harness_sets(root, paths)
    loaded = sorted({p for s in sets.values() for p in s})
    own = sum(size(root, p) for p in loaded)
    warnings = []
    file_max, warn_lines, per_token = budget["file_bytes_max"], budget["lines_warn"], budget["bytes_per_token"]
    limit_bytes = budget["combined_tokens_max"] * per_token
    global_bytes = budget["global_md_bytes"]

    for p in loaded:
        n = size(root, p)
        if n >= file_max:
            problems.append("%s: %d bytes, at or over the %d-byte cap a harness cuts files at; trim %d bytes "
                            "(move long procedures to a skill under .agents/skills/, history to docs/)"
                            % (p, n, file_max, n - file_max + 1))
        lines = read(root, p).count("\n")
        if lines > warn_lines:
            warnings.append("%s: %d lines, over the %d Claude Code advises for adherence" % (p, lines, warn_lines))
        if p not in STANDARD_FILES and p not in extra:
            harnesses = ", ".join(h for h in HARNESSES if p in sets[h])
            problems.append("%s: loaded at every session start (%s), and neither the standard nor "
                            "always_loaded_extra names it. Fold it into AGENTS.md, move it to a skill, or list "
                            "it with its reason: %s -d 'always_loaded_extra=[{\"path\": \"%s\", \"reason\": "
                            "\"<why every session needs it>\"}]' (keep the entries already listed)"
                            % (p, harnesses, UPDATE, p))
    for p in sorted(extra):
        if not extra[p].strip():
            problems.append(".agents/standard.yml: always_loaded_extra lists %s with no reason; give one" % p)

    for h in HARNESSES:
        repo_bytes = sum(size(root, p) for p in sets[h])
        total = global_bytes + repo_bytes
        if total > limit_bytes:
            problems.append("%s loads %d tokens at session start (global.md's budget of %d bytes plus %d bytes "
                            "here, at %d bytes a token), over the %d-token limit; trim %d bytes from %s"
                            % (h, -(-total // per_token), global_bytes, repo_bytes, per_token,
                               budget["combined_tokens_max"], total - limit_bytes, ", ".join(sets[h])))

    # A repo with no answers file (the template repo itself) has no ratchet to keep.
    if answered and ratchet is None:
        warnings.append("no always-loaded ratchet is set; record today's %d bytes: %s -d always_loaded_budget=%d"
                        % (own, UPDATE, own))
    elif answered and own > ratchet:
        problems.append("the always-loaded files (%s) total %d bytes, %d over this repo's ratchet of %d "
                        "(always_loaded_budget). Trim %d bytes, or raise the ratchet in this diff: %s "
                        "-d always_loaded_budget=%d"
                        % (", ".join(loaded), own, own - ratchet, ratchet, own - ratchet, UPDATE, own))
    return problems, warnings, own, source


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    root = argv[0] if argv else "."
    try:
        problems, warnings, own, source = check(root)
    except NotARepo:
        print("always-loaded: %s is not in a git repository; run it inside the repo" % os.path.abspath(root),
              file=sys.stderr)
        return 2
    for w in warnings:
        print("⚠️  always-loaded: " + w, file=sys.stderr)
    for p in problems:
        print("❌ " + p)
    if problems:
        print("always-loaded: %d problem(s) (budget from %s)" % (len(problems), source))
        return 1
    print("✅ always-loaded: %d bytes, within budget (from %s)" % (own, source))
    return 0


if __name__ == "__main__":
    sys.exit(main())
