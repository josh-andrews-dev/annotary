#!/usr/bin/env python3
"""Add the `test do` block to the Homebrew formula cargo-dist generated.

    python3 deploy/tap_formula.py path/to/annotary.rb

dist writes the formula with no test, and with an alias table and two one-caller helpers that
`brew audit --strict` refuses, so the tap's publish job (.github/workflows/publish-tap.yml) runs this
before the audit. It refuses a formula with a test already (one is all a formula may have), with no
closing `end` to put it before, with a download that has no checksum, or with helpers named as dist
names them but not in the shape it knows how to inline.
"""
import re
import sys

# dist's table of binary aliases, written so that `brew audit --strict` refuses it.
ALIASES = re.compile(r"^  BINARY_ALIASES = \{\n(?:    \"[^\"]+\": \{\},?\n)+  \}\n", re.M)

# dist's two helpers, each with one caller, which `brew audit --strict` wants inlined. Matched
# exactly: a helper of another shape is refused, not guessed at.
HELPERS = """  def target_triple
    cpu = Hardware::CPU.arm? ? "aarch64" : "x86_64"
    os = OS.mac? ? "apple-darwin" : "unknown-linux-gnu"

    "#{cpu}-#{os}"
  end

  def install_binary_aliases!
    BINARY_ALIASES[target_triple.to_sym].each do |source, dests|
      dests.each do |dest|
        bin.install_symlink bin/source.to_s => dest
      end
    end
  end

"""
HELPERS_CALL = "    install_binary_aliases!\n"
HELPERS_INLINED = """    cpu = Hardware::CPU.arm? ? "aarch64" : "x86_64"
    os = OS.mac? ? "apple-darwin" : "unknown-linux-gnu"
    BINARY_ALIASES[:"#{cpu}-#{os}"].each do |source, dests|
      dests.each do |dest|
        bin.install_symlink bin/source.to_s => dest
      end
    end
"""

TEST_BLOCK = """
  test do
    assert_match version.to_s, shell_output("#{bin}/annotary --version")
    ENV["ANNOTARY_SETUP_HANDLE"] = "brewtest"
    shell_output("#{bin}/annotary serve --setup-only --data-dir #{testpath}/data < /dev/null")
    assert_path_exists testpath/"data/store/annotary.sqlite"
  end
"""


def aligned_aliases(match):
    """The table as Homebrew's style wants it: aligned, a trailing comma, frozen."""
    names = re.findall(r'"([^"]+)": \{\}', match.group(0))
    width = max(len(n) for n in names) + 3
    rows = "".join('    %s {},\n' % ('"%s":' % n).ljust(width) for n in names)
    return "  BINARY_ALIASES = {\n" + rows + "  }.freeze\n"


def inlined_helpers(formula):
    """`formula` with dist's helpers inlined into `install`; ValueError if they are not dist's."""
    named = re.search(r"\b(target_triple|install_binary_aliases!)", formula)
    if not named:
        return formula
    if formula.count(HELPERS) != 1 or formula.count(HELPERS_CALL) != 1:
        raise ValueError("the formula's %s is not in the shape dist writes; inline it by hand" % named.group(1))
    formula = formula.replace(HELPERS, "").replace(HELPERS_CALL, HELPERS_INLINED)
    left = re.search(r"\b(target_triple|install_binary_aliases!)", formula)
    if left:
        raise ValueError("the formula still uses %s after inlining it" % left.group(1))
    return formula


def with_test(formula):
    """`formula` with the test added; ValueError if it should not be published."""
    if re.search(r"^\s*test do\b", formula, re.M):
        raise ValueError("the formula already has a test do block")
    urls = len(re.findall(r'^\s*url "', formula, re.M))
    sums = len(re.findall(r'^\s*sha256 "[0-9a-f]{64}"', formula, re.M))
    if not urls or urls != sums:
        raise ValueError("the formula has %d url lines and %d sha256 lines; every download needs a checksum" % (urls, sums))
    formula = ALIASES.sub(aligned_aliases, formula)
    formula = inlined_helpers(formula)
    # `brew audit --strict`: the version is already in the url.
    formula = re.sub(r'^  version "[^"]+"\n', "", formula, count=1, flags=re.M)
    stripped = formula.rstrip("\n")
    if not stripped.endswith("\nend"):
        raise ValueError("the formula does not end with the class's end")
    return stripped[: -len("end")].rstrip("\n") + "\n" + TEST_BLOCK + "end\n"


def main(argv):
    if len(argv) != 1:
        sys.exit(__doc__)
    with open(argv[0], encoding="utf-8") as f:
        formula = f.read()
    try:
        updated = with_test(formula)
    except ValueError as error:
        sys.exit("tap_formula: %s" % error)
    with open(argv[0], "w", encoding="utf-8") as f:
        f.write(updated)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
