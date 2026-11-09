"""Finds the owner's private names in a public repo's files, history and GitHub text.

`ci` scans what one GitHub event added and never prints the matched text: a public repo's logs
are public. `local` (`make private-check`) prints the exact words and scans the working tree, all
history and every GitHub text with its edit revisions. Exit: 0 clean, 1 findings, 2 cannot run.
"""
import base64
import fnmatch
import json
import os
import re
import subprocess
import sys
import traceback
import urllib.error
import urllib.parse
import urllib.request
from collections import namedtuple
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LOCAL_LIST = "~/.agents/private-terms.json"
EXCEPTIONS_FILE = ".agents/private-check-exceptions.json"
MISSING_SECRET = "PRIVATE_TERMS secret is not set; run `make private-terms-sync` in the owner's private config repo"
RULE = "public text names nothing of the owner's private setup"
POINTER = ("Run `make private-check` locally to see the exact words and every similar spot in this "
           "repo's files, history and GitHub text.")
REDACTED = "[redacted]"
COMMIT_FILE_CAP = 3000  # GitHub lists no more files than this for one commit, and gives no total.

# Each category's public-safe reason and fix. The terms themselves live only in the private list.
CATEGORIES = {
    "machine": ('evidence like "tests pass on <host>" is how a machine name usually slips in',
                'write "a Linux host", "a Mac" or "the server"'),
    "network": ("addresses and host names in logs, examples and evidence draw a map of a private network",
                'write "the LAN" or "the tailnet"; use 192.0.2.0/24 for examples'),
    "home-path": ("an absolute path copied from a terminal shows a real account name and folder layout",
                  "use ~ or $HOME"),
    "private-repo": ("a repo the public cannot see is private work, and a mention of it is a dead reference",
                     'say "a private repo" or omit it'),
    "profile": ("agent and profile handles identify the private automation behind the repo",
                'say "an agent"'),
    "email": ("a personal address in public text is harvested and cannot be taken back",
              "use a noreply or example.com address"),
    "person": ("a legal name outside the license ties the repo to a private person",
               "omit it"),
    "canary": ("a made-up term planted to test this check end to end",
               "none (test only)"),
}

Term = namedtuple("Term", "id category regex")
Waiver = namedtuple("Waiver", "where category")
Finding = namedtuple("Finding", "where category word term url")


class CannotRun(Exception):
    """A reason safe for a public log; the check neither passed nor failed on content."""


def load_terms(raw):
    try:
        data = json.loads(raw)
    except ValueError:
        raise CannotRun("the private list is not valid JSON") from None
    entries = data.get("entries", data.get("terms")) if isinstance(data, dict) else data
    if not isinstance(entries, list) or not entries:
        raise CannotRun("the private list has no entries")
    terms = []
    for number, entry in enumerate(entries, 1):
        if not isinstance(entry, dict) or not all(
                isinstance(entry.get(key), str) and entry[key] for key in ("id", "category", "pattern")):
            raise CannotRun("private list entry %d needs id, category and pattern" % number)
        if entry["category"] not in CATEGORIES:
            raise CannotRun("private list entry %d has an unknown category" % number)
        try:
            regex = re.compile(entry["pattern"], re.IGNORECASE)
        except re.error:
            raise CannotRun("private list entry %d has a pattern that does not compile" % number) from None
        terms.append(Term(entry["id"], entry["category"], regex))
    return terms


def load_exceptions():
    path = ROOT / EXCEPTIONS_FILE
    if not path.exists():
        return []
    try:
        entries = json.loads(path.read_text(encoding="utf-8"))["exceptions"]
    except (ValueError, KeyError, TypeError, OSError):
        raise CannotRun(EXCEPTIONS_FILE + " must be JSON with an `exceptions` list") from None
    found = []
    for number, entry in enumerate(entries, 1):
        complete = isinstance(entry, dict) and all(
            isinstance(entry.get(key), str) and entry[key].strip() for key in ("where", "category", "reason"))
        if not complete:
            raise CannotRun("%s entry %d needs where, category and a reason" % (EXCEPTIONS_FILE, number))
        if entry["category"] not in CATEGORIES:
            raise CannotRun("%s entry %d has an unknown category" % (EXCEPTIONS_FILE, number))
        found.append(Waiver(entry["where"], entry["category"]))
    return found


class Scanner:
    def __init__(self, terms, exceptions):
        self.terms = terms
        self.exceptions = exceptions
        self.findings = []
        self.seen = set()
        self.words = set()

    def excepted(self, where, path, category):
        places = {where, re.sub(r":\d+$", "", where)} | ({path} if path else set())
        return any(e.category == category and any(fnmatch.fnmatchcase(p, e.where) for p in places)
                   for e in self.exceptions)

    def line(self, where, line, path=None, url=None, categories=None):
        for term in self.terms:
            if categories and term.category not in categories:
                continue
            for match in term.regex.finditer(line):
                word = match.group(0)
                self.words.add(word)
                key = (where, term.category, word.lower())
                if word and key not in self.seen and not self.excepted(where, path, term.category):
                    self.seen.add(key)
                    self.findings.append(Finding(where, term.category, word, term.id, url))

    def text(self, where, text, url=None):
        for line in (text or "").splitlines():
            self.line(where, line, url=url)

    def public_where(self, where):
        """`where` with control characters out (a file name could start a workflow command) and any
        listed term redacted: a path or a release tag can itself be the finding. Every word matched
        so far, waived ones too, is removed as a whole token as well, because an anchored pattern
        that matched a bare path does not match the same text inside `path:line`. An anchored
        pattern's word embedded in a longer token stays: the token is not that word."""
        where = re.sub(r"[\x00-\x1f\x7f]", " ", where)
        for term in self.terms:
            where = term.regex.sub(REDACTED, where)
        for word in sorted(filter(None, self.words), key=len, reverse=True):
            where = re.sub(r"(?<![A-Za-z0-9])%s(?![A-Za-z0-9])" % re.escape(word), REDACTED, where, flags=re.IGNORECASE)
        return where


HUNK = re.compile(r"^@@ -\d+(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")


class Hunks:
    """Follows a unified diff. Line counts come from the hunk header, so an added line that looks
    like a file header (`+++ x`) is still read as an added line."""

    def __init__(self):
        self.old = self.new = self.number = 0

    @property
    def inside(self):
        return self.old > 0 or self.new > 0

    def feed(self, line):
        """The (new line number, text) of an added line, otherwise None."""
        if self.inside:
            mark = line[:1]
            if mark == "+":
                self.new -= 1
                self.number += 1
                return self.number - 1, line[1:]
            if mark == "-":
                self.old -= 1
            elif mark in (" ", ""):
                self.old -= 1
                self.new -= 1
                self.number += 1
            return None
        header = HUNK.match(line)
        if header:
            self.old = int(header.group(1) or 1)
            self.number = int(header.group(2))
            self.new = int(header.group(3) or 1)
        return None


class GitHub:
    def __init__(self, token):
        self.token = token
        self.base = os.environ.get("GITHUB_API_URL", "https://api.github.com").rstrip("/")

    def call(self, url, payload=None):
        data = None if payload is None else json.dumps(payload).encode()
        request = urllib.request.Request(url, data=data, headers={
            "Accept": "application/vnd.github+json", "User-Agent": "private-check",
            "X-GitHub-Api-Version": "2022-11-28", "Authorization": "Bearer " + self.token})
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                return json.load(response), response.headers.get("Link", "")
        except urllib.error.HTTPError as err:
            raise CannotRun("GitHub answered HTTP %d reading %s" % (err.code, urllib.parse.urlsplit(url).path)) from None
        except (urllib.error.URLError, OSError, ValueError):
            raise CannotRun("could not read %s from GitHub" % urllib.parse.urlsplit(url).path) from None

    def get(self, path):
        return self.call(self.base + path)[0]

    def pages(self, path):
        """Each page of a REST list or object that paginates (a commit's `files`)."""
        url = "%s%s%sper_page=100" % (self.base, path, "&" if "?" in path else "?")
        while url:
            page, link = self.call(url)
            yield page
            following = re.search(r'<([^>]+)>;\s*rel="next"', link)
            url = following.group(1) if following else None

    def graphql(self, query, variables):
        answer = self.call(self.base + "/graphql", {"query": query, "variables": variables})[0]
        if answer.get("errors") or not answer.get("data"):
            detail = (answer.get("errors") or [{}])[0].get("message", "no data")
            raise CannotRun("GitHub GraphQL failed: %s" % detail)
        return answer["data"]


def scan_commit_text(scanner, sha, commit, url=None):
    scanner.text("commit %s message" % sha, commit.get("message"), url=url)
    # Only the email category: the author's name is on every commit by design.
    for role in ("author", "committer"):
        email = (commit.get(role) or {}).get("email") or ""
        scanner.line("commit %s %s email" % (sha, role), email, url=url, categories={"email"})


def scan_file_change(gh, repo, change, scanner):
    """The added lines of one REST file entry. The entry comes from GitHub's API, so nothing of the
    pull request is ever checked out; a diff too large for `patch` is read as a blob, still data."""
    path, status = change["filename"], change.get("status")
    if status == "removed":
        return
    scanner.line("%s (file name)" % path, path, path=path)
    patch = change.get("patch")
    if patch:
        hunks = Hunks()
        for raw in patch.split("\n"):
            added = hunks.feed(raw)
            if added:
                scanner.line("%s:%d" % (path, added[0]), added[1], path=path)
    else:
        # No patch means too large, binary, or a rename or copy: read them all, since a NUL byte
        # or an unchanged rename says nothing about the text inside.
        blob = gh.get("/repos/%s/git/blobs/%s" % (repo, change["sha"]))
        raw = base64.b64decode(blob.get("content") or "") if blob.get("encoding") == "base64" else None
        if raw is None or len(raw) != blob.get("size"):
            raise CannotRun("could not read a changed file's whole contents from GitHub")
        for number, line in enumerate(raw.decode("utf-8", "replace").splitlines(), 1):
            scanner.line("%s:%d" % (path, number), line, path=path)


def require_all(got, expected, what):
    """GitHub stops listing past a cap; a pull request beyond it must not pass half-read."""
    if isinstance(expected, int) and got < expected:
        raise CannotRun("GitHub listed only %d of %d %s" % (got, expected, what))


def scan_pull_request(gh, repo, event, scanner):
    pr = event["pull_request"]
    number = pr["number"]
    scanner.text("PR #%d title" % number, pr.get("title"))
    scanner.text("PR #%d body" % number, pr.get("body"))
    commits = [c for page in gh.pages("/repos/%s/pulls/%d/commits" % (repo, number)) for c in page]
    require_all(len(commits), pr.get("commits"), "commits")
    for commit in commits:
        scan_commit_text(scanner, commit["sha"], commit["commit"])
    files = [f for page in gh.pages("/repos/%s/pulls/%d/files" % (repo, number)) for f in page]
    require_all(len(files), pr.get("changed_files"), "changed files")
    for change in files:
        scan_file_change(gh, repo, change, scanner)


def pushed_shas(gh, repo, event):
    """Every commit a push added. The event's own list is capped, so a push with a known start is
    listed through the compare API and checked against its total, and a new branch has no start,
    so all of its history is listed."""
    before = event.get("before") or ""
    if not before.strip("0"):
        pages = gh.pages("/repos/%s/commits?sha=%s" % (repo, event["after"]))
        return [c["sha"] for page in pages for c in page]
    shas, total = [], None
    for page in gh.pages("/repos/%s/compare/%s...%s" % (repo, before, event["after"])):
        shas += [c["sha"] for c in page["commits"]]
        total = page["total_commits"]
    require_all(len(shas), total, "pushed commits")
    return shas


def scan_push(gh, repo, event, scanner):
    if event.get("deleted"):
        return
    for sha in pushed_shas(gh, repo, event):
        files = []
        for page in gh.pages("/repos/%s/commits/%s" % (repo, sha)):
            commit, files = page["commit"], files + page.get("files", [])
        if len(files) >= COMMIT_FILE_CAP:
            raise CannotRun("commit %s reaches GitHub's limit of %d listed files" % (sha, COMMIT_FILE_CAP))
        scan_commit_text(scanner, sha, commit)
        for change in files:
            scan_file_change(gh, repo, change, scanner)


def scan_issue(event, scanner):
    issue = event["issue"]
    scanner.text("issue #%d title" % issue["number"], issue.get("title"))
    scanner.text("issue #%d body" % issue["number"], issue.get("body"))


def scan_issue_comment(event, scanner):
    issue = event["issue"]
    label = "PR" if issue.get("pull_request") else "issue"
    scanner.text("%s #%d comment %d" % (label, issue["number"], event["comment"]["id"]), event["comment"].get("body"))


def scan_review_comment(event, scanner):
    scanner.text("PR #%d review comment %d" % (event["pull_request"]["number"], event["comment"]["id"]),
                 event["comment"].get("body"))


def scan_release(event, scanner):
    release = event["release"]
    tag = release.get("tag_name") or str(release.get("id"))
    scanner.line("release %s tag" % tag, release.get("tag_name") or "")
    scanner.text("release %s name" % tag, release.get("name"))
    scanner.text("release %s body" % tag, release.get("body"))


def run_ci():
    raw = os.environ.get("PRIVATE_TERMS", "")
    if not raw.strip():
        raise CannotRun(MISSING_SECRET)
    scanner = Scanner(load_terms(raw), load_exceptions())
    name, repo = os.environ["GITHUB_EVENT_NAME"], os.environ["GITHUB_REPOSITORY"]
    with open(os.environ["GITHUB_EVENT_PATH"], encoding="utf-8") as f:
        event = json.load(f)
    gh = GitHub(os.environ.get("GITHUB_TOKEN", ""))
    if name in ("pull_request", "pull_request_target"):
        scan_pull_request(gh, repo, event, scanner)
    elif name == "push":
        scan_push(gh, repo, event, scanner)
    elif name == "issues":
        scan_issue(event, scanner)
    elif name == "issue_comment":
        scan_issue_comment(event, scanner)
    elif name == "pull_request_review_comment":
        scan_review_comment(event, scanner)
    elif name == "release":
        scan_release(event, scanner)
    else:
        raise CannotRun("no scan is written for the event " + name)
    return report_ci(scanner)


def write_summary(text):
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if path:
        with open(path, "a", encoding="utf-8") as f:
            f.write("```text\n%s\n```\n" % text)


def annotate(message):
    print("::error title=private-check::" + message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A"))


def report_ci(scanner):
    if not scanner.findings:
        print("private-check: clean.")
        return 0
    # The where and category only: a word, a pattern, an entry id or a note would be a public leak.
    places = []
    for finding in scanner.findings:
        place = (scanner.public_where(finding.where), finding.category)
        if place not in places:
            places.append(place)
    blocks = ["private-check: %d finding(s). The matched text is not shown: this log is public." % len(places)]
    for where, category in places:
        why, fix = CATEGORIES[category]
        blocks.append("where: %s\ncategory: %s\nrule: %s\nwhy: %s\nfix: %s\n%s" % (where, category, RULE, why, fix, POINTER))
    text = "\n\n".join(blocks)
    print(text)
    write_summary(text)
    for where, category in places[:10]:
        annotate("%s: %s" % (where, category))
    return 1


def git(*args):
    result = subprocess.run(["git", *args], cwd=ROOT, capture_output=True)
    if result.returncode != 0:
        raise CannotRun("git %s failed" % args[0])
    return result.stdout


def scan_tree(scanner):
    for rel in git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split(b"\0"):
        path = rel.decode("utf-8", "replace")
        full = ROOT / path
        if not path or not (full.is_file() or full.is_symlink()):
            continue
        scanner.line("%s (file name)" % path, path, path=path)
        raw = os.readlink(full).encode() if full.is_symlink() else full.read_bytes()
        for number, line in enumerate(raw.decode("utf-8", "replace").splitlines(), 1):
            scanner.line("%s:%d" % (path, number), line, path=path)


def scan_history(scanner, repo):
    fields = ["%H", "%ae", "%ce", "%B"]
    command = ["git", "-c", "core.quotepath=off", "log", "--all", "-p", "--no-color", "--no-ext-diff",
               "--no-textconv", "--format=%x01" + "%x1f".join(fields) + "%x02"]
    process = subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    sha, url, path, header = None, None, None, None
    hunks = Hunks()
    for raw in process.stdout:
        line = raw.decode("utf-8", "replace").rstrip("\n")
        if header is None and line.startswith("\x01"):
            header = ""
            line = line[1:]
        if header is not None:
            header += line + "\n"
            if "\x02" in line:
                sha, author, committer, message = header.split("\x02")[0].split("\x1f", 3)
                url = "https://github.com/%s/commit/%s" % (repo, sha) if repo else None
                scan_commit_text(scanner, sha, {"message": message, "author": {"email": author},
                                                "committer": {"email": committer}}, url=url)
                header, path, hunks = None, None, Hunks()
            continue
        added = hunks.feed(line)
        if added and path:
            scanner.line("commit %s %s:%d" % (sha, path, added[0]), added[1], path=path, url=url)
        elif not hunks.inside and line.startswith("+++ "):
            name = line[4:].rstrip("\t").strip('"')
            path = name[2:] if name.startswith("b/") else None
            if path:
                scanner.line("commit %s %s (file name)" % (sha, path), path, path=path, url=url)
    if process.wait() != 0:
        raise CannotRun("git log failed")


PAGE = "pageInfo{hasNextPage endCursor}"
EDIT_FIELDS = "diff editedAt"
TEXT_FIELDS = "id url body userContentEdits(first:100){%s nodes{%s}}" % (PAGE, EDIT_FIELDS)
COMMENT_FIELDS = "databaseId " + TEXT_FIELDS


def walk(gh, query, variables, keys):
    """Every node of a GraphQL connection found under `keys`."""
    after = None
    while True:
        data = gh.graphql(query, dict(variables, after=after))
        for key in keys:
            data = data[key]
        yield from data["nodes"]
        if not data["pageInfo"]["hasNextPage"]:
            return
        after = data["pageInfo"]["endCursor"]


def on_repo(gh, repo, field, selection):
    owner, name = repo.split("/", 1)
    query = ("query($owner:String!,$name:String!,$after:String){repository(owner:$owner,name:$name)"
             "{%s(first:100,after:$after){%s nodes{%s}}}}" % (field, PAGE, selection))
    return walk(gh, query, {"owner": owner, "name": name}, ("repository", field))


def on_node(gh, node_id, kind, field, selection):
    query = ("query($id:ID!,$after:String){node(id:$id){... on %s{%s(first:100,after:$after){%s nodes{%s}}}}}"
             % (kind, field, PAGE, selection))
    return walk(gh, query, {"id": node_id}, ("node", field))


def scan_text_node(gh, scanner, where, node):
    scanner.text(where, node.get("body"), url=node["url"])
    edits = node["userContentEdits"]
    revisions = edits["nodes"]
    if edits["pageInfo"]["hasNextPage"]:
        revisions = list(on_node(gh, node["id"], "Comment", "userContentEdits", EDIT_FIELDS))
    for revision in revisions:
        if revision.get("diff"):
            scanner.text("%s revision %s" % (where, revision["editedAt"]), revision["diff"], url=node["url"])


def scan_github_text(gh, repo, scanner):
    for kind, label, field in (("PullRequest", "PR", "pullRequests"), ("Issue", "issue", "issues")):
        for item in on_repo(gh, repo, field, "number title " + TEXT_FIELDS):
            prefix = "%s #%d" % (label, item["number"])
            scanner.text(prefix + " title", item["title"], url=item["url"])
            scan_text_node(gh, scanner, prefix + " body", item)
            for comment in on_node(gh, item["id"], kind, "comments", COMMENT_FIELDS):
                scan_text_node(gh, scanner, "%s comment %d" % (prefix, comment["databaseId"]), comment)
            if kind != "PullRequest":
                continue
            for review in on_node(gh, item["id"], kind, "reviews", COMMENT_FIELDS):
                scan_text_node(gh, scanner, "%s review %d" % (prefix, review["databaseId"]), review)
                for comment in on_node(gh, review["id"], "PullRequestReview", "comments", COMMENT_FIELDS):
                    scan_text_node(gh, scanner, "%s review comment %d" % (prefix, comment["databaseId"]), comment)
    for release in on_repo(gh, repo, "releases", "name tagName description url"):
        tag = release["tagName"]
        scanner.line("release %s tag" % tag, tag, url=release["url"])
        scanner.text("release %s name" % tag, release.get("name"), url=release["url"])
        scanner.text("release %s body" % tag, release.get("description"), url=release["url"])


def local_repo():
    if os.environ.get("GITHUB_REPOSITORY"):
        return os.environ["GITHUB_REPOSITORY"]
    remote = subprocess.run(["git", "remote", "get-url", "origin"], cwd=ROOT, capture_output=True, text=True)
    match = re.search(r"github\.com[:/]([^/]+/[^/]+?)(?:\.git)?\s*$", remote.stdout)
    return match.group(1) if match else None


def local_token():
    if os.environ.get("GITHUB_TOKEN"):
        return os.environ["GITHUB_TOKEN"]
    try:
        result = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True)
    except OSError:
        result = None
    if result is None or result.returncode != 0 or not result.stdout.strip():
        raise CannotRun("no GitHub login to read the repo's text; run `gh auth login`, or pass --no-github")
    return result.stdout.strip()


def run_local(no_github):
    raw = os.environ.get("PRIVATE_TERMS", "")
    if not raw.strip():
        path = Path(LOCAL_LIST).expanduser()
        if not path.exists():
            raise CannotRun("the private list is not on this machine (%s); only the owner's machines have it" % LOCAL_LIST)
        raw = path.read_text(encoding="utf-8")
    scanner = Scanner(load_terms(raw), load_exceptions())
    repo = local_repo()
    scan_tree(scanner)
    scan_history(scanner, repo)
    if not no_github:
        if not repo:
            raise CannotRun("origin is not a GitHub repo; pass --no-github to scan files and history only")
        scan_github_text(GitHub(local_token()), repo, scanner)
    return report_local(scanner, no_github)


def report_local(scanner, no_github):
    if not scanner.findings:
        print("private-check: clean (files, history%s)." % ("" if no_github else ", GitHub text"))
        return 0
    print("private-check: %d finding(s). This output holds the exact words: keep it out of public places." % len(scanner.findings))
    for finding in scanner.findings:
        print("%s: %s: %r [%s]%s" % (finding.where, finding.category, finding.word, finding.term,
                                     " " + finding.url if finding.url else ""))
    print()
    for category in sorted({f.category for f in scanner.findings}):
        why, fix = CATEGORIES[category]
        print("%s: %s; fix: %s" % (category, why, fix))
    return 1


def main(argv):
    mode = argv[1] if len(argv) > 1 else ""
    try:
        if mode == "ci":
            return run_ci()
        if mode == "local":
            return run_local("--no-github" in argv[2:])
        raise CannotRun("usage: private_check.py ci | local [--no-github]")
    except CannotRun as err:
        print("private-check: %s" % err, file=sys.stderr)
        if mode == "ci":
            annotate(str(err))
            write_summary(str(err))
        return 2
    except Exception as err:
        # An exception's text can hold what it was reading, so a public log gets the type only.
        if mode == "local":
            traceback.print_exc()
        else:
            print("private-check: stopped on %s" % type(err).__name__, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
