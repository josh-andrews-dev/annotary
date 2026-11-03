#!/usr/bin/env python3
"""Refuse British spellings and en-GB locales on changed lines. Exit 0 clean, 1 findings, 2 cannot check.

    python3 scripts/standard/en_us.py [repo-root]              changed lines (make lint, make ci)
    python3 scripts/standard/en_us.py --all [repo-root]        the whole tree, to find what to clean up
    python3 scripts/standard/en_us.py --base REF [repo-root]   changed lines since the merge base with REF

Product text is American English, and text already in the repo never blocks: only lines changed
since the merge base with the pull request's base (GITHUB_BASE_REF in CI) or origin/main count,
uncommitted and untracked work included. With no origin/main, as before a repo's first push, every
line is new. A rename git detects (at least half the file unchanged) counts only for the lines it changed.

Spellings come from codespell's en-GB_to_en-US dictionary, pinned below and run through uvx, plus
EXTRA_WORDS for what it misses. codespell reads `colour_map`, `backgroundColour` and `colour-map`
as one word, so identifiers do not fire. If uvx is missing or codespell cannot run, this fails: it
never passes without checking.

Exceptions belong to the repo, in .agents/en-us-exceptions.json; every entry needs a reason:

    {"words": [{"word": "colour", "reason": "the name of a third-party API field"}],
     "paths": [{"path": "src/voice.ts", "reason": "the spoken voice stays en-GB, by choice"}]}

A word entry also takes a locale ("en-GB"). A path is a file, a folder or an fnmatch pattern from
the repo root, and every check skips it. Skipped outright: docs/archive/, lock files, vendored and
generated files (including .gitattributes' linguist-vendored and linguist-generated), symlinks and
binaries. UTF-16 text cannot be read, so changing it is a finding, not a skip.

To use it in a repo not on the template: copy this one file to any folder (it finds the repo root
itself), add the exceptions file if the repo needs one, run it from wherever the repo's checks run,
with uv on the PATH and CI checking out full history (fetch-depth: 0). Set --base if the default
branch is not main.

Stdlib only; runs on Python 3.9.
"""
import fnmatch
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

CODESPELL_VERSION = "2.4.3"
EXCEPTIONS = ".agents/en-us-exceptions.json"
# What codespell's dictionary misses, each with its American form.
EXTRA_WORDS = {
    "whilst": "while", "amongst": "among", "learnt": "learned", "unlearnt": "unlearned",
    "spelt": "spelled", "dreamt": "dreamed", "leapt": "leaped",
    "notarise": "notarize", "notarised": "notarized", "notarises": "notarizes", "notarising": "notarizing",
    "notarisation": "notarization", "notarisations": "notarizations",
}
# What a word is, for codespell and for EXTRA_WORDS, so an identifier is one word to both. Codespell's
# default also counts an apostrophe as part of a word, so it misses 'colour' in a single-quoted string.
WORD_PATTERN = r"[\w\-]+"
WORD = re.compile(WORD_PATTERN)
# A locale literal, in any case, including `en-GB-SoniaNeural`, but not inside `en_GB_voice`.
LOCALE = re.compile(r"(?<![\w-])en[-_]gb(?!\w)", re.IGNORECASE)
PRAGMA = "codespell:ignore"
FINDING = re.compile(r"^(.+?):(\d+): (.+?) ==> (.*)$")
BATCH = 200     # files per codespell run, well under any command-line limit
TIMEOUT = 300   # seconds per codespell run, the download included

LOCK_NAMES = {"Cargo.lock", "Gemfile.lock", "Package.resolved", "Podfile.lock", "bun.lock", "bun.lockb",
              "composer.lock", "flake.lock", "go.sum", "npm-shrinkwrap.json", "package-lock.json",
              "pnpm-lock.yaml", "poetry.lock", "uv.lock", "yarn.lock"}
# Any folder of these names, anywhere in the repo.
SKIP_FOLDERS = {"node_modules", "vendor", "vendored", "third_party", "third-party", "Pods", "Carthage",
                "DerivedData", ".build", "generated", "__generated__", ".venv", "venv"}
# Only at the root: a source folder deeper down may have one of these names.
SKIP_ROOT_FOLDERS = {"build", "dist"}
SKIP_PREFIXES = ("docs/archive/",
                 # The template's copies of a skill's checkers, pinned by hash and never edited in a repo.
                 "scripts/design/")
GENERATED_NAME = re.compile(r"\.(min\.(js|css)|map|pbxproj)$|\.generated\.|\.g\.dart$|\.pb\.go$|_pb2\.py$")
GENERATED_MARK = re.compile(rb"@generated|Code generated .* DO NOT EDIT")
UTF16_BOMS = (b"\xff\xfe", b"\xfe\xff")


class CannotCheck(Exception):
    """The check could not run, which is never a pass."""


def git(root, *args):
    return subprocess.run(["git", "-C", root] + list(args), capture_output=True)


def git_out(root, *args):
    result = git(root, *args)
    if result.returncode != 0:
        raise CannotCheck("`git %s` failed: %s" % (" ".join(args), result.stderr.decode(errors="replace").strip()))
    return result.stdout.decode(errors="surrogateescape")


def merge_base(root, ref, hint):
    if git(root, "rev-parse", "-q", "--verify", ref + "^{commit}").returncode != 0:
        raise CannotCheck("cannot find %s to compare with; %s" % (ref, hint))
    result = git(root, "merge-base", ref, "HEAD")
    if result.returncode != 0:
        raise CannotCheck("no merge base between %s and HEAD. A shallow clone cannot have one: in CI, check out "
                          "with fetch-depth: 0; locally, run `git fetch --unshallow`" % ref)
    return result.stdout.decode().strip()


def find_base(root, base):
    """The commit changed lines are counted from, and how to describe it."""
    if base:
        return merge_base(root, base, "check the --base value"), base
    ci_base = os.environ.get("GITHUB_BASE_REF")
    if ci_base:
        ref = "origin/" + ci_base
        return merge_base(root, ref, "the CI checkout needs fetch-depth: 0"), ref
    if git(root, "rev-parse", "-q", "--verify", "origin/main^{commit}").returncode == 0:
        return merge_base(root, "origin/main", ""), "origin/main"
    empty = git_out(root, "hash-object", "-t", "tree", os.devnull).strip()
    print("en-US check: no origin/main, so every line counts as new (a default branch with another name: --base)",
          file=sys.stderr)
    return empty, "an empty tree"


def unquote_path(path):
    """A path as `git diff` prints it: a trailing tab when it has a space, C quotes when it has a control
    character or a double quote."""
    path = path[:-1] if path.endswith("\t") else path
    if not path.startswith('"'):
        return path
    names = {"a": 7, "b": 8, "t": 9, "n": 10, "v": 11, "f": 12, "r": 13, '"': 34, "\\": 92}
    raw = path[1:-1].encode("utf-8", "surrogateescape")
    out = bytearray()
    i = 0
    while i < len(raw):
        if raw[i] != 92:
            out.append(raw[i])
            i += 1
        elif chr(raw[i + 1]) in names:
            out.append(names[chr(raw[i + 1])])
            i += 2
        else:
            out.append(int(raw[i + 1:i + 4], 8))
            i += 4
    return out.decode("utf-8", "surrogateescape")


def changed_lines(root, base):
    """{path: set of changed line numbers} between `base` and the working tree, untracked files whole.
    --text gives hunks for what git calls binary too: a UTF-16 file or a text file under a -diff
    attribute changes by line, and a mode change or an exact rename has no hunk. read_text then
    drops the real binaries."""
    patch = git_out(root, "-c", "core.quotePath=false", "diff", "--text", "--unified=0", "--no-color", "--no-ext-diff",
                    "--no-textconv", "--find-renames", "--diff-filter=d", "--ignore-submodules=all",
                    "--src-prefix=a/", "--dst-prefix=b/", base, "--")
    changed = {}
    path = None
    body = 0        # lines of the current hunk still to pass: a removed `++ x` looks like a file header
    for line in patch.split("\n"):
        if body:
            if not line.startswith("\\"):
                body -= 1
            continue
        if line.startswith("+++ "):
            name = unquote_path(line[4:])
            path = name[2:] if name.startswith("b/") else None
            continue
        m = re.match(r"^@@ -\d+(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", line)
        if m and path:
            removed = int(m.group(1)) if m.group(1) is not None else 1
            added = int(m.group(3)) if m.group(3) is not None else 1
            changed.setdefault(path, set()).update(range(int(m.group(2)), int(m.group(2)) + added))
            body = removed + added
    for path in git_out(root, "ls-files", "--others", "--exclude-standard", "-z").split("\0"):
        if path:
            changed[path] = None    # every line
    return {p: lines for p, lines in changed.items() if lines is None or lines}


def all_lines(root):
    out = git_out(root, "ls-files", "--cached", "--others", "--exclude-standard", "-z")
    return {p: None for p in out.split("\0") if p}


def load_exceptions(root):
    """The exceptions file as ({lower-case word}, [path patterns]). A malformed one stops the check."""
    full = os.path.join(root, EXCEPTIONS)
    if not os.path.isfile(full):
        return set(), []

    def bad(why):
        return CannotCheck("%s: %s. It holds {\"words\": [{\"word\": ..., \"reason\": ...}], "
                           "\"paths\": [{\"path\": ..., \"reason\": ...}]}" % (EXCEPTIONS, why))

    try:
        with open(full, encoding="utf-8") as f:
            data = json.load(f)
    except ValueError as e:
        raise bad("not valid JSON (%s)" % e)
    if not isinstance(data, dict) or set(data) - {"about", "words", "paths"}:
        raise bad("an object with only about, words and paths")
    found = {}
    for section, key in (("words", "word"), ("paths", "path")):
        entries = data.get(section, [])
        if not isinstance(entries, list):
            raise bad("%s is not a list" % section)
        found[section] = []
        for entry in entries:
            if not isinstance(entry, dict) or set(entry) != {key, "reason"}:
                raise bad("each entry in %s has exactly %s and reason: %s" % (section, key, json.dumps(entry)))
            if not (isinstance(entry[key], str) and entry[key].strip()):
                raise bad("%s in %s is empty: %s" % (key, section, json.dumps(entry)))
            if not (isinstance(entry["reason"], str) and entry["reason"].strip()):
                raise bad("an exception needs a reason: %s" % json.dumps(entry))
            found[section].append(entry[key].strip())
    return {w.lower() for w in found["words"]}, found["paths"]


def path_excepted(path, patterns):
    return any(path == p or path.startswith(p.rstrip("/") + "/") or fnmatch.fnmatchcase(path, p) for p in patterns)


def attribute_skips(root, paths):
    """Paths .gitattributes marks linguist-generated or linguist-vendored."""
    if not paths:
        return set()
    result = subprocess.run(["git", "-C", root, "check-attr", "-z", "--stdin", "linguist-generated", "linguist-vendored"],
                            input="\0".join(paths).encode("utf-8", "surrogateescape") + b"\0", capture_output=True)
    if result.returncode != 0:
        raise CannotCheck("`git check-attr` failed: %s" % result.stderr.decode(errors="replace").strip())
    fields = result.stdout.decode("utf-8", "surrogateescape").split("\0")
    return {fields[i] for i in range(0, len(fields) - 2, 3) if fields[i + 2] in ("set", "true")}


def skipped_by_name(path, own):
    parts = path.split("/")
    name = parts[-1]
    return (path in own
            or path.startswith(SKIP_PREFIXES)
            or name in LOCK_NAMES or name.endswith((".lock", "-lock.json", "-lock.yaml"))
            or parts[0] in SKIP_ROOT_FOLDERS and len(parts) > 1
            or any(p in SKIP_FOLDERS for p in parts[:-1])
            or GENERATED_NAME.search(name) is not None)


def is_utf16_text(data):
    """A UTF-16 byte order mark, and what follows reads as mostly-ASCII text; a binary can start the same way."""
    if not data.startswith(UTF16_BOMS):
        return False
    try:
        text = data.decode("utf-16")
    except UnicodeDecodeError:
        return False
    if any(ord(c) < 32 and c not in "\t\n\r\f\v" for c in text):
        return False
    return sum(ord(c) < 127 for c in text) * 4 >= len(text)


def read_text(root, path):
    """The file's bytes, or None for what is not a readable text file in its own right."""
    full = os.path.join(root, path)
    if os.path.islink(full) or not os.path.isfile(full):
        return None
    with open(full, "rb") as f:
        data = f.read()
    if is_utf16_text(data):
        return data
    if b"\0" in data[:8000] or GENERATED_MARK.search(data[:1000]):
        return None
    return data


def run_codespell(paths):
    """{(absolute path, line): [(word, American form)]}. Runs from an empty folder, so a repo's own
    codespell config cannot change what is found."""
    uvx = shutil.which("uvx")
    command = [uvx, "codespell==" + CODESPELL_VERSION, "--builtin", "en-GB_to_en-US", "--check-hidden",
               "--disable-colors", "--regex", WORD_PATTERN]
    found = {}
    with tempfile.TemporaryDirectory(prefix="en-us-") as empty:
        for start in range(0, len(paths), BATCH):
            batch = paths[start:start + BATCH]
            for attempt in (1, 2):      # a download from PyPI can fail once
                try:
                    result = subprocess.run(command + batch, cwd=empty, capture_output=True, text=True,
                                            errors="surrogateescape", timeout=TIMEOUT)
                    why = "exit %d: %s" % (result.returncode, result.stderr.strip()[-400:])
                    if result.returncode in (0, 65):
                        break
                except subprocess.TimeoutExpired:
                    why = "no result after %d seconds" % TIMEOUT
                if attempt == 2:
                    raise CannotCheck("codespell %s did not run through uvx (%s)" % (CODESPELL_VERSION, why))
                time.sleep(2)
            for line in result.stdout.splitlines():
                m = FINDING.match(line)
                if not m or m.group(1) not in batch:
                    raise CannotCheck("cannot read codespell's output line: %s" % line)
                found.setdefault((m.group(1), int(m.group(2))), []).append(
                    (m.group(3), m.group(4).split(" | ")[0].strip().rstrip(",")))
    return found


def with_case_of(word, form):
    if word.isupper():
        return form.upper()
    return form[0].upper() + form[1:] if word[0].isupper() else form


def line_findings(text, skip_words):
    """(what, message) for the checks made here, not by codespell, on one line of text."""
    out = []
    for m in WORD.finditer(text):
        form = EXTRA_WORDS.get(m.group(0).lower())
        if form and m.group(0).lower() not in skip_words:
            out.append('"%s": write "%s"' % (m.group(0), with_case_of(m.group(0), form)))
    for m in LOCALE.finditer(text):
        if m.group(0).lower() not in skip_words:
            out.append('"%s": a locale literal changes dates and numbers in the product; write "en-US" '
                       '(with hour12: false where it was only a way to get 24-hour time)' % m.group(0))
    if PRAGMA in text:
        out.append('"%s": this check does not honor it; correct the word, or add an exception with a reason' % PRAGMA)
    return out


def scan(root, targets):
    """Findings as sorted (path, line, message), given {path: changed line numbers, or None for all}."""
    skip_words, skip_paths = load_exceptions(root)
    own = {EXCEPTIONS, ".agents/rules/standard.md"}     # the check's own text names the words it refuses
    here = os.path.realpath(__file__)
    real_root = os.path.realpath(root)
    if here.startswith(real_root + os.sep):
        own.add(os.path.relpath(here, real_root).replace(os.sep, "/"))
    paths = [p for p in sorted(targets) if not skipped_by_name(p, own) and not path_excepted(p, skip_paths)]
    marked = attribute_skips(root, paths)
    texts = {}
    for p in paths:
        if p not in marked:
            data = read_text(root, p)
            if data is not None:
                texts[p] = data
    findings = set()
    for path in [p for p, data in texts.items() if is_utf16_text(data)]:
        del texts[path]
        findings.add((path, 1, '"UTF-16": this check cannot read it; save the file as UTF-8, or add a path exception'))
    for path, data in texts.items():
        wanted = targets[path]
        for n, line in enumerate(data.split(b"\n"), 1):
            if wanted is None or n in wanted:
                for message in line_findings(line.decode("utf-8", "replace"), skip_words):
                    findings.add((path, n, message))
    by_full = {os.path.join(real_root, p): p for p in texts}
    if by_full:
        for (full, n), hits in run_codespell(sorted(by_full)).items():
            path = by_full[full]
            if targets[path] is None or n in targets[path]:
                for word, form in hits:
                    if word.lower() not in skip_words:
                        findings.add((path, n, '"%s": write "%s"' % (word, form)))
    return sorted(findings), len(texts)


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    whole = "--all" in argv
    argv = [a for a in argv if a != "--all"]
    base = None
    if "--base" in argv:
        at = argv.index("--base")
        if at + 1 >= len(argv):
            print("usage: en_us.py [--all] [--base REF] [repo-root]", file=sys.stderr)
            return 2
        base = argv[at + 1]
        del argv[at:at + 2]
    if len(argv) > 1 or (argv and argv[0].startswith("-")) or (whole and base):
        print("usage: en_us.py [--all] [--base REF] [repo-root]", file=sys.stderr)
        return 2
    try:
        top = git(argv[0] if argv else ".", "rev-parse", "--show-toplevel")
        if top.returncode != 0:
            raise CannotCheck("%s is not in a git repository; run it inside the repo"
                              % os.path.abspath(argv[0] if argv else "."))
        root = top.stdout.decode(errors="surrogateescape").strip()
        if not shutil.which("uvx"):
            raise CannotCheck("uvx is not on the PATH. Install uv (https://docs.astral.sh/uv/); in a GitHub-hosted "
                              "CI job add the astral-sh/setup-uv step. This check never passes without running")
        if whole:
            targets, where = all_lines(root), "the whole tree"
        else:
            commit, ref = find_base(root, base)
            targets, where = changed_lines(root, commit), "lines changed since " + ref
        findings, count = scan(root, targets)
    except CannotCheck as e:
        print("❌ en-US check could not run, so it fails: %s" % e, file=sys.stderr)
        return 2
    if not findings:
        print("✅ en-US check: clean (%s, %d file(s))" % (where, count))
        return 0
    for path, n, message in findings:
        print("❌ %s:%d: %s" % (path, n, message))
    words = [m.split('"')[1] for _, _, m in findings if not m.startswith(('"UTF-16"', '"' + PRAGMA))]
    print("en-US check: %d finding(s) in %s. Product text is American English (en-US).\n"
          "A deliberate exception goes in %s with a reason: under \"words\", "
          "{\"word\": \"%s\", \"reason\": \"...\"}; under \"paths\", {\"path\": \"%s\", \"reason\": \"...\"}."
          % (len(findings), where, EXCEPTIONS, words[0] if words else "...", findings[0][0]))
    return 1


if __name__ == "__main__":
    sys.exit(main())
