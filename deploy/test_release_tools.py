#!/usr/bin/env python3
"""deploy/stamp_version.py, deploy/tap_formula.py and verify_live.py's browser path; `make deploy-test` runs this."""
import importlib.util
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


stamp_version = load("stamp_version")
tap_formula = load("tap_formula")
verify_live = load("verify_live")

MANIFEST = """[workspace]
members = ["crates/*"]

[workspace.package]
# A placeholder: the comment stays.
version = "0.0.0"
edition = "2024"

[workspace.dependencies]
other = { version = "0.0.0" }
"""
LOCK = """version = 4

[[package]]
name = "annotary"
version = "0.0.0"
dependencies = [
 "annotary-store",
]

[[package]]
name = "annotary-store"
version = "0.0.0"

[[package]]
name = "annotation-thing"
version = "0.0.0"

[[package]]
name = "annotary-store-extra-thing"
version = "0.0.0"
"""
FORMULA = """class Annotary < Formula
  desc "Share Markdown"
  url "https://example.com/a.tar.gz"
  sha256 "%s"

  def install
    bin.install "annotary"
  end
end
""" % ("a" * 64)
# The formula dist generated for v0.7.1, as released.
with open(os.path.join(HERE, "..", "fixtures", "release", "dist-annotary-v0.7.1.rb"), encoding="utf-8") as f:
    DIST_FORMULA = f.read()


class Stamp(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.write("Cargo.toml", MANIFEST)
        self.write("Cargo.lock", LOCK)

    def write(self, name, text):
        with open(os.path.join(self.dir.name, name), "w", encoding="utf-8") as f:
            f.write(text)

    def read(self, name):
        with open(os.path.join(self.dir.name, name), encoding="utf-8") as f:
            return f.read()

    def test_it_stamps_the_workspace_version_and_its_crates_and_nothing_else(self):
        self.assertEqual(stamp_version.stamp(self.dir.name, "v1.2.3"), 2)
        manifest = self.read("Cargo.toml")
        self.assertIn('[workspace.package]\n# A placeholder: the comment stays.\nversion = "1.2.3"\n', manifest)
        self.assertIn('other = { version = "0.0.0" }', manifest)
        lock = self.read("Cargo.lock")
        self.assertIn('name = "annotary"\nversion = "1.2.3"', lock)
        self.assertIn('name = "annotary-store"\nversion = "1.2.3"', lock)
        self.assertIn('name = "annotation-thing"\nversion = "0.0.0"', lock)
        self.assertIn('name = "annotary-store-extra-thing"\nversion = "0.0.0"', lock)

    def test_it_refuses_what_is_not_a_plain_version_and_writes_nothing(self):
        for bad in ("1.2.3", "v1.2", "v1.2.3-rc.1", "annotary-v1.2.3", ""):
            with self.assertRaises(ValueError, msg=bad):
                stamp_version.stamp(self.dir.name, bad)
        self.assertEqual(self.read("Cargo.toml"), MANIFEST)

    def test_it_refuses_a_manifest_with_no_placeholder_and_a_lock_with_no_crate(self):
        self.write("Cargo.toml", MANIFEST.replace('version = "0.0.0"\nedition', 'version = "9.9.9"\nedition'))
        with self.assertRaises(ValueError):
            stamp_version.stamp(self.dir.name, "v1.2.3")
        self.write("Cargo.toml", MANIFEST)
        self.write("Cargo.lock", "version = 4\n")
        with self.assertRaises(ValueError):
            stamp_version.stamp(self.dir.name, "v1.2.3")
        self.assertEqual(self.read("Cargo.toml"), MANIFEST)


class Formula(unittest.TestCase):
    def test_the_test_block_goes_inside_the_class(self):
        out = tap_formula.with_test(FORMULA)
        self.assertTrue(out.endswith('  end\nend\n'), out)
        self.assertIn("  end\n\n  test do\n", out)
        for needle in ("annotary --version", "serve --setup-only", "#{testpath}/data"):
            self.assertIn(needle, out)
        self.assertEqual(out.count("test do"), 1)

    def test_what_homebrew_audit_refuses_in_dists_formula_is_rewritten(self):
        formula = FORMULA.replace('  url "', '  version "1.2.3"\n  url "', 1).replace(
            "  def install",
            '  BINARY_ALIASES = {\n    "aarch64-apple-darwin": {},\n    "x86_64-unknown-linux-gnu": {}\n  }\n\n  def install',
        )
        out = tap_formula.with_test(formula)
        self.assertNotIn('version "1.2.3"', out)
        self.assertIn(
            '  BINARY_ALIASES = {\n    "aarch64-apple-darwin":     {},\n    "x86_64-unknown-linux-gnu": {},\n  }.freeze\n',
            out,
        )

    def test_an_alias_table_it_does_not_recognize_passes_through_for_the_audit_to_judge(self):
        table = '  BINARY_ALIASES = {\n    "aarch64-apple-darwin": {"annotary": ["an"]},\n  }\n\n'
        out = tap_formula.with_test(FORMULA.replace("  def install", table + "  def install"))
        self.assertIn(table, out)

    def test_dists_one_caller_helpers_are_inlined_into_install(self):
        out = tap_formula.with_test(DIST_FORMULA)
        for gone in ("def target_triple", "def install_binary_aliases!", "target_triple", "install_binary_aliases!"):
            self.assertNotIn(gone, out)
        self.assertIn(
            '    bin.install "annotary"\n    end\n\n'
            '    cpu = Hardware::CPU.arm? ? "aarch64" : "x86_64"\n'
            '    os = OS.mac? ? "apple-darwin" : "unknown-linux-gnu"\n'
            '    BINARY_ALIASES[:"#{cpu}-#{os}"].each do |source, dests|\n'
            "      dests.each do |dest|\n"
            "        bin.install_symlink bin/source.to_s => dest\n"
            "      end\n"
            "    end\n\n"
            "    # Homebrew will automatically install these",
            out,
        )
        self.assertIn("  }.freeze\n\n  def install\n", out)
        self.assertNotIn('  version "0.7.1"', out)
        self.assertEqual(out.count("  def "), 1)
        self.assertEqual(out.count("test do"), 1)

    def test_helpers_it_does_not_recognize_are_refused(self):
        for bad in (
            DIST_FORMULA.replace('"aarch64" : "x86_64"', '"arm64" : "x86_64"'),
            DIST_FORMULA.replace("    install_binary_aliases!\n", ""),
            DIST_FORMULA.replace("    install_binary_aliases!\n", "    install_binary_aliases!\n" * 2),
            DIST_FORMULA.replace("  def install\n", "  def install\n    ohai target_triple\n"),
            DIST_FORMULA.replace("    install_binary_aliases!\n", "    if true\n      install_binary_aliases!\n    end\n"),
            DIST_FORMULA.replace("    install_binary_aliases!\n", "    #    install_binary_aliases!\n"),
        ):
            with self.assertRaises(ValueError, msg=bad):
                tap_formula.with_test(bad)

    def test_it_refuses_a_formula_that_should_not_go_out(self):
        for bad in (
            tap_formula.with_test(FORMULA),  # a second test
            FORMULA.replace("a" * 64, "not-a-checksum"),
            FORMULA.replace('  sha256 "%s"\n' % ("a" * 64), ""),
            FORMULA.replace('  url "https://example.com/a.tar.gz"\n', "").replace('  sha256 "%s"\n' % ("a" * 64), ""),
            FORMULA[: -len("end\n")],
        ):
            with self.assertRaises(ValueError, msg=bad):
                tap_formula.with_test(bad)


class BrowsersPath(unittest.TestCase):
    """Playwright's default browser folder per platform (verify_live passes it on, since the
    scenarios run with HOME set to the copy's folder)."""

    def test_macos_uses_library_caches(self):
        self.assertEqual(verify_live.browsers_path({}, "darwin", "/h"), "/h/Library/Caches/ms-playwright")

    def test_linux_uses_the_xdg_cache(self):
        self.assertEqual(verify_live.browsers_path({}, "linux", "/h"), "/h/.cache/ms-playwright")
        self.assertEqual(verify_live.browsers_path({"XDG_CACHE_HOME": "/x"}, "linux", "/h"), "/x/ms-playwright")

    def test_windows_uses_local_app_data(self):
        self.assertEqual(verify_live.browsers_path({"LOCALAPPDATA": "/l"}, "win32", "/h"), os.path.join("/l", "ms-playwright"))

    def test_a_set_path_wins_on_every_platform(self):
        for platform in ("darwin", "linux", "win32"):
            self.assertEqual(verify_live.browsers_path({"PLAYWRIGHT_BROWSERS_PATH": "/p"}, platform, "/h"), "/p")


if __name__ == "__main__":
    unittest.main()
