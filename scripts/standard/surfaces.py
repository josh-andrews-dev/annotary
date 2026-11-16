#!/usr/bin/env python3
"""The repo's surfaces: what it ships to, declared in `.agents/standard.yml`. Exit 1 on a finding.

    python3 scripts/standard/surfaces.py lint [repo-root]
    PR_TITLE=... PR_BODY=... python3 scripts/standard/surfaces.py pr [repo-root]

The answers file holds two lists, written by `copier update -d`:

    surfaces:                      # what the repo ships to; two or more turn the `surfaces` check on
    -   name: Web
        signs: [web/, web-v*]      # the signs below that belong to this surface
    not_surfaces:                  # signs that are not a surface, each with why
    -   sign: tools/
        reason: scripts only the maintainer runs

`lint` (run by `make lint`, so by CI) finds signs of a surface and fails on each one neither list
names. The signs, the same whatever the list holds:
- folder: a top-level app, firmware or client folder (SIGN_FOLDERS), or a folder under apps/ or
  clients/: `ios/`, `apps/admin/`;
- tag: a release tag series other than plain `v`, from a *_TAG_PREFIX in Makefile or project.mk or
  from the repo's tags: `ios-v*`;
- workflow: a workflow whose file name says deploy, release, publish or testflight;
- client: a package manifest below the root in a folder named client or sdk (a client of the
  repo's API): `packages/api-client/`. One inside a folder sign, another client, or a folder the
  lists name is that folder's.
It also fails when two or more surfaces are recorded but the template's surfaces workflow is not
there (the list was edited by hand, not with `copier update -d`).

`pr` is the `surfaces` workflow's check: a pull request titled feat, fix or perf has a
`## Surfaces` section in its body with one line per surface, `changed`, `not affected: <why>` or
`follows: <reason>`, where the reason is a design answer from the owner, an outside contributor's
pull request, or an urgent fix to a live server. Other titles pass.

Stdlib only; runs on Python 3.9.
"""
import json
import os
import re
import subprocess
import sys

ANSWERS = ".agents/standard.yml"
WORKFLOW = ".github/workflows/surfaces.yml"
UPDATE = ("uvx copier update --trust --defaults --vcs-ref=:current: --answers-file .agents/standard.yml "
          "-d 'surfaces=[{\"name\": \"...\", \"signs\": [\"...\"]}]' "
          "-d 'not_surfaces=[{\"sign\": \"...\", \"reason\": \"...\"}]' (each -d gives the whole list)")
# Top-level folders that hold a deliverable of their own. apps/ and clients/ hold one per child.
SIGN_FOLDERS = {"android", "app", "cli", "client", "desktop", "extension", "firmware", "ios", "mac",
                "macos", "mobile", "site", "watchos", "web", "webapp"}
PARENT_FOLDERS = {"apps", "clients"}
MANIFESTS = {"package.json", "pyproject.toml", "setup.py", "Cargo.toml", "Package.swift", "go.mod",
             "build.gradle", "build.gradle.kts"}
CLIENT_NAME = re.compile(r"(?:^|[-_.])(?:client|sdk)s?(?:$|[-_.])", re.I)
NOT_SHIPPED = {"node_modules", "vendor", "third_party", "test", "tests", "fixtures", "examples", "example", "docs"}
DEPLOY_WORKFLOW = re.compile(r"^\.github/workflows/[^/]*(?:deploy|release|publish|testflight)[^/]*\.ya?ml$", re.I)
TAG_PREFIX_LINE = re.compile(r"^\s*[A-Z][A-Z0-9_]*TAG_PREFIX\s*(?::=|\?=|=)\s*(\S+)\s*(?:#.*)?$")
TAG = re.compile(r"^([a-z][a-z0-9-]*-v)\d+\.\d+\.\d+")
SIGN_SYNTAX = re.compile(r"^(?:[\w.-]+/)+$|^[a-z][a-z0-9-]*-v\*$|^\.github/workflows/[\w.-]+\.ya?ml$")
# A name the pull request line can carry: no colon, no Markdown emphasis.
NAME_SYNTAX = re.compile(r"^[A-Za-z0-9][A-Za-z0-9 ./+&-]*$")
CLIENT = "a client of the repo's API"
TITLES = re.compile(r"^(feat|fix|perf)(\([^)]*\))?!?:")
FOLLOWS = re.compile(r"\b(design|contributor|urgent)", re.I)


class NotARepo(Exception):
    pass


def files(root):
    result = subprocess.run(
        ["git", "-C", root, "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        capture_output=True)
    if result.returncode != 0:
        raise NotARepo(root)
    return sorted(p for p in result.stdout.decode().split("\0") if p and os.path.isfile(os.path.join(root, p)))


def read(root, path):
    with open(os.path.join(root, path), encoding="utf-8", errors="replace") as f:
        return f.read()


# ── the answers file: a reader for the block YAML Copier's to_nice_yaml writes ───────────────

def scalar(text):
    text = text.strip()
    if text[:1] == "'" and text[-1:] == "'" and len(text) >= 2:
        return text[1:-1].replace("''", "'")
    if text[:1] == '"' and text[-1:] == '"' and len(text) >= 2:
        try:
            return json.loads(text)
        except ValueError:
            return text[1:-1]
    if text[:1] == "[" and text[-1:] == "]":
        inner = text[1:-1].strip()
        return [scalar(part) for part in inner.split(",")] if inner else []
    if text == "{}":
        return {}
    if text in ("null", "Null", "NULL", "~", ""):
        return None     # Copier writes `signs: null` for a null list
    return text


def uncomment(line):
    """The line without a trailing `# comment`; a `#` inside quotes, or not after a space, stays."""
    quote, i = None, -1
    while i + 1 < len(line):
        i += 1
        c = line[i]
        if quote:
            if c == quote == "'" and line[i + 1:i + 2] == "'":
                i += 1      # '' is one literal quote inside single quotes
            elif c == quote:
                quote = None
        elif c in "'\"" and (i == 0 or line[i - 1] in " \t[,:-"):
            quote = c
        elif c == "#" and (i == 0 or line[i - 1] in " \t"):
            return line[:i].rstrip()
    return line


def is_item(text):
    return text == "-" or text.startswith("- ")


KEY = re.compile(r"^([A-Za-z_][\w-]*):(?:\s+(.*))?$")


def parse(lines, pos, indent):
    """(value, next position) of the block at `indent`; `lines` is a list of [indent, text]."""
    if is_item(lines[pos][1]):
        items = []
        while pos < len(lines) and lines[pos][0] == indent and is_item(lines[pos][1]):
            rest = lines[pos][1][1:]
            body = rest.lstrip()
            if not body:
                pos += 1
                if pos < len(lines) and lines[pos][0] > indent:
                    value, pos = parse(lines, pos, lines[pos][0])
                else:
                    value = None
            else:
                col = indent + 1 + len(rest) - len(body)
                lines[pos] = [col, body]
                value, pos = parse(lines, pos, col)
            items.append(value)
        return items, pos
    if KEY.match(lines[pos][1]):
        mapping = {}
        while pos < len(lines) and lines[pos][0] == indent and not is_item(lines[pos][1]):
            m = KEY.match(lines[pos][1])
            if not m:
                raise ValueError(lines[pos][1])
            pos += 1
            if m.group(2):
                mapping[m.group(1)] = scalar(m.group(2))
            elif pos < len(lines) and (lines[pos][0] > indent or (lines[pos][0] == indent and is_item(lines[pos][1]))):
                mapping[m.group(1)], pos = parse(lines, pos, lines[pos][0])
            else:
                mapping[m.group(1)] = None
        return mapping, pos
    return scalar(lines[pos][1]), pos + 1


def answer(text, key):
    """The value of a top-level key, or [] when the file has none."""
    block, inside = [], False
    for line in text.splitlines():
        line = uncomment(line.replace("\t", " "))
        if line.strip() and not line[0].isspace() and not line.startswith("-"):
            inside = line.startswith(key + ":")
        if inside and line.strip():
            block.append([len(line) - len(line.lstrip()), line.strip()])
    if not block:
        return []
    value, pos = parse(block, 0, 0)
    if pos < len(block):
        # A line the blocks above did not take (mixed indentation): never a silently shorter list.
        raise ValueError("unexpected indentation at %r" % block[pos][1])
    return value.get(key) or []


def declarations(root):
    """(surfaces, not_surfaces, problems), each list as recorded; problems name a malformed entry."""
    if not os.path.isfile(os.path.join(root, ANSWERS)):
        return [], [], []
    text = read(root, ANSWERS)
    problems = []
    try:
        surfaces, marked = answer(text, "surfaces"), answer(text, "not_surfaces")
    except (ValueError, IndexError, AttributeError) as e:
        return [], [], ["%s: surfaces or not_surfaces cannot be read (%s); record them again with %s" % (ANSWERS, e, UPDATE)]
    if not isinstance(surfaces, list) or not isinstance(marked, list):
        return [], [], ["%s: surfaces and not_surfaces are lists; record them again with %s" % (ANSWERS, UPDATE)]
    seen = set()
    for s in surfaces:
        name = s.get("name") if isinstance(s, dict) else None
        if not isinstance(name, str) or not NAME_SYNTAX.match(name):
            problems.append("%s: the surface %r needs a name of letters, digits, spaces, dots, slashes, plus, "
                            "& or dashes (no colon)" % (ANSWERS, s))
            continue
        if name.lower() in seen:
            problems.append("%s: the surface %s is named twice" % (ANSWERS, name))
        seen.add(name.lower())
        signs = s.get("signs") or []
        if not isinstance(signs, list) or not all(isinstance(x, str) for x in signs):
            problems.append("%s: the surface %s's signs are a list of strings" % (ANSWERS, name))
    for m in marked:
        if not isinstance(m, dict) or not isinstance(m.get("sign"), str):
            problems.append("%s: the not_surfaces entry %r needs a sign" % (ANSWERS, m))
        elif not isinstance(m.get("reason"), str) or not re.search(r"\w.*\w", m["reason"]):
            problems.append("%s: not a surface: %s needs the reason why" % (ANSWERS, m["sign"]))
    for sign in sorted({x for s in surfaces if isinstance(s, dict) and isinstance(s.get("signs"), list)
                        for x in s["signs"] if isinstance(x, str)} |
                       {m["sign"] for m in marked if isinstance(m, dict) and isinstance(m.get("sign"), str)}):
        if not SIGN_SYNTAX.match(sign):
            problems.append("%s: %r is not a sign: a folder (`web/`), a tag series (`web-v*`) or a "
                            "workflow (`.github/workflows/deploy.yml`)" % (ANSWERS, sign))
    return surfaces, marked, problems


def names(surfaces):
    return [s["name"] for s in surfaces if isinstance(s, dict) and isinstance(s.get("name"), str)]


# ── lint ────────────────────────────────────────────────────────────────────────────────────

def tags(root):
    out = subprocess.run(["git", "-C", root, "tag", "-l"], capture_output=True).stdout.decode("utf-8", "replace")
    return out.split()


def signs(root, paths):
    """{sign: what kind of sign it is}."""
    found = {}
    for p in paths:
        parts = p.split("/")
        if len(parts) >= 2 and parts[0].lower() in SIGN_FOLDERS:
            found[parts[0] + "/"] = "an app, firmware or client folder"
        if len(parts) >= 3 and parts[0].lower() in PARENT_FOLDERS:
            found["/".join(parts[:2]) + "/"] = "an app or client folder"
        if DEPLOY_WORKFLOW.match(p):
            found[p] = "a deploy or release workflow"
    folders = list(found)
    for p in sorted(paths, key=lambda p: p.count("/")):     # a shallower client holds a deeper one
        parts = p.split("/")
        folder = "/".join(parts[:-1]) + "/"
        if (parts[-1] in MANIFESTS and 2 <= len(parts) <= 4
                and not any(x.lower() in NOT_SHIPPED for x in parts[:-1])
                and any(CLIENT_NAME.search(x) for x in parts[:-1])
                and not any(folder.startswith(f) for f in folders if f.endswith("/"))):
            found[folder] = CLIENT
            folders.append(folder)
    prefixes = set()
    for mk in ("Makefile", "project.mk"):
        if mk in paths:
            for line in read(root, mk).splitlines():
                m = TAG_PREFIX_LINE.match(line)
                if m:
                    prefixes.add(m.group(1))
    prefixes |= {m.group(1) for m in map(TAG.match, tags(root)) if m}
    for prefix in prefixes:
        if prefix != "v" and re.match(r"^[a-z][a-z0-9-]*-v$", prefix):
            found[prefix + "*"] = "a release tag series"
    return found


def lint(root):
    paths = files(root)
    if ANSWERS not in paths:
        return []
    surfaces, marked, problems = declarations(root)
    declared = {x for s in surfaces if isinstance(s, dict) and isinstance(s.get("signs"), list)
                for x in s["signs"] if isinstance(x, str)}
    excused = {m["sign"] for m in marked if isinstance(m, dict) and isinstance(m.get("sign"), str)}
    for sign in sorted(declared & excused):
        problems.append("%s: %s is both a surface's sign and marked not a surface; keep one" % (ANSWERS, sign))
    undeclared = 0
    covered = [f for f in declared | excused if f.endswith("/")]
    for sign, kind in sorted(signs(root, paths).items()):
        if kind == CLIENT and any(sign.startswith(f) for f in covered):
            continue    # a client inside a folder the repo declared or marked is that folder's
        if sign not in declared and sign not in excused:
            undeclared += 1
            problems.append("surfaces: %s (%s) is not declared: declare it in surfaces:, or mark it not a "
                            "surface: <why> in %s" % (sign, kind, ANSWERS))
    if undeclared:
        problems.append("surfaces: record them with %s" % UPDATE)
    if len(names(surfaces)) >= 2 and WORKFLOW not in paths:
        problems.append("%s: %d surfaces are recorded but %s is missing; record them with copier update -d, "
                        "never by hand, so the template renders the surfaces check: %s"
                        % (ANSWERS, len(names(surfaces)), WORKFLOW, UPDATE))
    return problems


# ── pr ──────────────────────────────────────────────────────────────────────────────────────

def section(body):
    """The prose lines of the body's `## Surfaces` section: HTML comments (the template's hints)
    and fenced code are not text, and a heading inside a fence is not one."""
    text = re.sub(r"<!--[\s\S]*?(?:-->|$)", "", body.replace("\r", ""))
    lines, inside, fence = [], False, None
    for line in text.split("\n"):
        f = re.match(r"^ {0,3}(`{3,}|~{3,})", line)
        if fence:
            if f and f.group(1)[0] == fence[0] and len(f.group(1)) >= len(fence):
                fence = None
            continue
        if f:
            fence = f.group(1)
            continue
        h = re.match(r"^ {0,3}(#{1,6})\s+(.*?)\s*#*\s*$", line)
        if h and len(h.group(1)) <= 2:
            inside = len(h.group(1)) == 2 and h.group(2).lower() == "surfaces"
            continue
        if inside:
            lines.append(line)
    return lines


def check_pr(title, body, surface_names):
    """The problems with a pull request's `## Surfaces` section; [] when it passes."""
    if not TITLES.match(title):
        return []
    how = ('Add a "## Surfaces" section with one line per surface (%s), each "changed", '
           '"not affected: <why>" or "follows: <reason>", e.g. "- %s: not affected: nothing there shows this".'
           % (", ".join(surface_names), surface_names[0] if surface_names else "Web"))
    lines = section(body)
    if not any(l.strip() for l in lines):
        return ["A %s pull request needs a ## Surfaces section. %s" % (re.split(r"[(:!]", title)[0], how)]
    problems = []
    for name in surface_names:
        key = re.compile(r"^\s{0,3}(?:[-*+]\s+)?(?:\*\*|__)?" + r"\s+".join(map(re.escape, name.split()))
                         + r"(?:\*\*|__)?\s*:(?:\*\*|__)?\s*(.*)$", re.I)
        hits = [m for m in map(key.match, lines) if m]
        if len(hits) != 1:
            problems.append("%s: say it once, not %d times" % (name, len(hits)) if hits else "%s: missing" % name)
            continue
        value = re.sub(r"[*_`]", "", hits[0].group(1)).strip()
        if re.match(r"^changed\b", value, re.I):
            continue
        m = re.match(r"^not affected\s*:\s*(.*)$", value, re.I)
        if m:
            if not re.search(r"[A-Za-z0-9].*[A-Za-z0-9]", m.group(1)):
                problems.append('%s: "not affected:" needs the reason why' % name)
            continue
        m = re.match(r"^follows\s*:\s*(.*)$", value, re.I)
        if m:
            if not FOLLOWS.search(m.group(1)):
                problems.append('%s: "follows:" is allowed only for a design answer from the owner, an outside '
                                "contributor's pull request, or an urgent fix to a live server; say which" % name)
            continue
        problems.append('%s: say "changed", "not affected: <why>" or "follows: <reason>"' % name)
    if problems:
        problems.append(how)
    return problems


def pr(root):
    surfaces, _, found = declarations(root)
    if not found:
        found = check_pr(os.environ.get("PR_TITLE", ""), os.environ.get("PR_BODY", ""), names(surfaces))
    if found:
        msg = "## Surfaces: " + "; ".join(found) + " Edit the body; the check runs again."
        print("::error title=surfaces::" + msg)
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            with open(summary, "a") as f:
                f.write(msg + "\n")
    return found


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if not argv or argv[0] not in ("lint", "pr"):
        print("usage: surfaces.py lint|pr [repo-root]", file=sys.stderr)
        return 2
    root = argv[1] if len(argv) > 1 else "."
    if argv[0] == "pr":
        problems = pr(root)
        print("surfaces: %d problem(s)" % len(problems) if problems else "✅ surfaces: every surface is accounted for")
        return 1 if problems else 0
    try:
        problems = lint(root)
    except NotARepo:
        print("surfaces: %s is not in a git repository; run it inside the repo" % os.path.abspath(root), file=sys.stderr)
        return 2
    for p in problems:
        print("❌ " + p)
    print("surfaces: %d problem(s)" % len(problems) if problems else "✅ surfaces: every sign is declared")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
