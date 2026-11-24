"""Self-test of make_notices.py (no cargo call).

    py -3 -B tools\\test_make_notices.py

Builds fixture cargo metadata and fixture crate folders in a temp folder.
The last test checks the real THIRD-PARTY-NOTICES.txt at the repository root
(when it exists) for the Windows user name.
"""

import io
import json
import os
import shutil
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import make_notices as mn  # noqa: E402

MIT_A = "MIT License\r\n\r\nCopyright (c) 2020 Someone   \r\n\r\nPermission is hereby granted.\r\n"
MIT_B = "MIT License\n\nCopyright (c) 2020 Someone\n\nPermission is hereby granted.\n\n\n"
APACHE = "Apache License\nVersion 2.0, January 2004\n"
OFL = "SIL OPEN FONT LICENSE Version 1.1\n"


class Fixture:
    """Fixture crate folders plus the cargo metadata that names them."""

    def __init__(self, base):
        self.base = base
        self.packages = []
        self.nodes = {}
        self.root_id = self.add("canoxide", "0.4.0", "MIT", files={"LICENSE": "root licence\n"},
                                folder=os.path.join(base, "ws"))

    def add(self, name, version, licence, files=None, folder=None, license_file=None, repo=None):
        folder = folder or os.path.join(base_reg(self.base), "%s-%s" % (name, version))
        os.makedirs(folder, exist_ok=True)
        for rel, text in (files or {}).items():
            path = os.path.join(folder, *rel.split("/"))
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "wb") as f:
                f.write(text.encode("utf-8") if isinstance(text, str) else text)
        with open(os.path.join(folder, "Cargo.toml"), "w", encoding="utf-8") as f:
            f.write("[package]\nname = %r\n" % name)
        pid = "registry+https://example.invalid/index#%s@%s" % (name, version)
        self.packages.append({
            "name": name, "version": version, "id": pid, "license": licence,
            "license_file": license_file, "repository": repo or "https://example.invalid/" + name,
            "manifest_path": os.path.join(folder, "Cargo.toml"),
        })
        self.nodes[pid] = {"id": pid, "deps": []}
        return pid

    def dep(self, frm, to, *kinds):
        self.nodes[frm]["deps"].append({
            "name": to.split("#")[1].split("@")[0], "pkg": to,
            "dep_kinds": [{"kind": k, "target": None} for k in kinds],
        })

    def meta(self):
        return {
            "packages": self.packages,
            "workspace_members": [self.root_id],
            "workspace_root": os.path.join(self.base, "ws"),
            "resolve": {"root": self.root_id, "nodes": list(self.nodes.values())},
        }

    def write_meta(self):
        path = os.path.join(self.base, "meta.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(self.meta(), f)
        return path


def base_reg(base):
    return os.path.join(base, "registry")


def run_main(*args):
    out, err = io.StringIO(), io.StringIO()
    with redirect_stdout(out), redirect_stderr(err):
        code = mn.main(list(args))
    return code, out.getvalue(), err.getvalue()


class NoticesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="notices_test_")
        fx = Fixture(self.tmp)
        r = fx.root_id
        self.a = fx.add("alpha", "1.0.0", "MIT", files={"LICENSE-MIT": MIT_A})
        self.b = fx.add("beta", "2.0.0", "MIT OR Apache-2.0",
                        files={"license-mit": MIT_B, "LICENSE-APACHE": APACHE})
        self.c = fx.add("gamma", "0.1.0", "Apache-2.0")  # no licence file
        self.d = fx.add("devonly", "3.0.0", "MIT", files={"LICENSE": "dev only text\n"})
        self.e = fx.add("buildtool", "0.5.0", "Zlib", files={"COPYING": "zlib text\n"})
        self.f9 = fx.add("multi", "1.9.0", "ISC", files={"LICENSE": "isc 1.9\n"})
        self.f10 = fx.add("multi", "1.10.0", "ISC", files={"LICENSE": "isc 1.10\n"})
        self.g = fx.add("custom", "1.0.0", None, files={"docs/TERMS.txt": "custom terms\n"},
                        license_file="docs/TERMS.txt")
        self.h = fx.add("reuse", "1.0.0", "MIT", files={"LICENSES/MIT.txt": "reuse mit\n"})
        self.fonts = fx.add("epaint_default_fonts", "0.31.1", "OFL-1.1",
                            files={"fonts/Foo-Regular.ttf": b"\0\1\0\0",
                                   "fonts/OFL.txt": OFL, "fonts/README.rs": "not a licence"})
        self.p = fx.add("egui-phosphor", "0.9.0", "MIT OR Apache-2.0", files={"LICENSE-MIT": MIT_B})
        fx.dep(r, self.a, None)
        fx.dep(r, self.b, None)
        fx.dep(r, self.d, "dev")
        fx.dep(r, self.e, "build")
        fx.dep(r, self.fonts, None)
        fx.dep(r, self.p, None)
        fx.dep(self.a, self.c, None)
        fx.dep(self.a, self.f10, None)
        fx.dep(self.b, self.f9, None)
        fx.dep(self.b, self.g, None)
        fx.dep(self.b, self.h, None)
        self.fx = fx
        self.meta_path = fx.write_meta()
        self.out = os.path.join(self.tmp, "out", "NOTICES.txt")
        os.makedirs(os.path.dirname(self.out))

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def render(self):
        return mn.render(self.fx.meta())

    def test_dev_dependency_left_out(self):
        text = self.render()
        self.assertNotIn("devonly", text)
        self.assertNotIn("dev only text", text)

    def test_build_dependency_kept(self):
        text = self.render()
        self.assertIn("buildtool", text)
        self.assertIn("zlib text", text)
        self.assertIn("buildtool 0.5.0 (COPYING)", text)

    def test_root_not_listed(self):
        text = self.render()
        self.assertNotIn("root licence", text)
        self.assertIn("Crates: 10", text)

    def test_identical_texts_merged(self):
        text = self.render()
        self.assertEqual(text.count("Permission is hereby granted."), 1)
        i = text.index("Permission is hereby granted.")
        head = text[:i]
        block = head[head.rindex("Used by:"):]
        self.assertIn("alpha 1.0.0 (LICENSE-MIT)", block)
        self.assertIn("beta 2.0.0 (license-mit)", block)
        self.assertIn("egui-phosphor 0.9.0 (LICENSE-MIT)", block)

    def test_crate_without_licence_file(self):
        text = self.render()
        sec = text[text.index("3. NO LICENCE FILE IN THE PACKAGE"):text.index("4. FONTS AND ICONS")]
        self.assertIn("gamma", sec)
        self.assertIn("Apache-2.0", sec)
        self.assertIn("printed in section 2", sec)
        self.assertNotIn("alpha", sec)

    def test_license_file_and_licenses_folder(self):
        text = self.render()
        self.assertIn("custom 1.0.0 (docs/TERMS.txt)", text)
        self.assertIn("custom terms", text)
        self.assertIn("reuse 1.0.0 (LICENSES/MIT.txt)", text)
        self.assertIn("(none)", text)  # custom has no licence expression

    def test_sorted_by_name_then_version(self):
        text = self.render()
        summary = text[text.index("1. SUMMARY"):text.index("2. LICENCE TEXTS")]
        self.assertLess(summary.index("1.9.0"), summary.index("1.10.0"))
        lines = summary.splitlines()
        start = next(i for i, ln in enumerate(lines) if ln.startswith("-----")) + 1
        rows = lines[start:lines.index("", start)]
        names = [ln.split()[0] for ln in rows]
        self.assertEqual(len(rows), 10)
        self.assertEqual(names, sorted(names))

    def test_fonts_section(self):
        text = self.render()
        sec = text[text.index("4. FONTS AND ICONS"):]
        self.assertIn("Inter", sec)
        self.assertIn("JetBrains Mono", sec)
        self.assertIn("egui-phosphor", sec)
        self.assertIn("font     Foo-Regular.ttf", sec)
        self.assertIn("licence  OFL.txt", sec)
        self.assertIn("SIL OPEN FONT LICENSE Version 1.1", sec)
        self.assertNotIn("README.rs", sec)

    def test_deterministic(self):
        code, _, _ = run_main("--metadata", self.meta_path, "--out", self.out, "--quiet")
        self.assertEqual(code, 0)
        with open(self.out, "rb") as f:
            first = f.read()
        code, _, _ = run_main("--metadata", self.meta_path, "--out", self.out, "--quiet")
        with open(self.out, "rb") as f:
            second = f.read()
        self.assertEqual(first, second)
        self.assertNotIn(b"\r", first)
        self.assertTrue(first.endswith(b"\n") and not first.endswith(b"\n\n"))

    def test_no_absolute_path(self):
        text = self.render()
        low = text.lower()
        for v in (self.tmp, self.tmp.replace("\\", "/"), os.path.expanduser("~")):
            self.assertNotIn(v.lower(), low)
        self.assertNotIn(":\\", text)

    def test_path_guard(self):
        meta = self.fx.meta()
        folder = os.path.join(self.tmp, "registry", "alpha-1.0.0")
        with open(os.path.join(folder, "LICENSE-MIT"), "w") as f:
            f.write("see " + folder.replace("\\", "/") + "/README\n")
        with self.assertRaises(RuntimeError):
            mn.render(meta)

    def test_check(self):
        code, _, err = run_main("--check", "--metadata", self.meta_path, "--out", self.out)
        self.assertEqual(code, 1, "missing file")
        self.assertIn("missing", err)
        self.assertEqual(run_main("--metadata", self.meta_path, "--out", self.out, "--quiet")[0], 0)
        self.assertEqual(run_main("--check", "--metadata", self.meta_path, "--out", self.out, "--quiet")[0], 0)
        with open(self.out, "ab") as f:
            f.write(b"x")
        code, _, err = run_main("--check", "--metadata", self.meta_path, "--out", self.out)
        self.assertEqual(code, 1)
        self.assertIn("out of date", err)

    def test_uncommon(self):
        self.assertFalse(mn.uncommon("MIT OR Apache-2.0"))
        self.assertFalse(mn.uncommon("MIT/Apache-2.0"))
        self.assertFalse(mn.uncommon("Apache-2.0 WITH LLVM-exception OR MIT"))
        self.assertTrue(mn.uncommon("0BSD OR MIT"))
        self.assertTrue(mn.uncommon(None))


class RealOutputTest(unittest.TestCase):
    """The generated file at the repository root holds no user name."""

    def test_no_user_name(self):
        path = mn.DEFAULT_OUT
        if not os.path.isfile(path):
            self.skipTest("THIRD-PARTY-NOTICES.txt not generated yet")
        user = os.environ.get("USERNAME", "")
        if len(user) < 3:
            self.skipTest("no usable USERNAME")
        with open(path, "rb") as f:
            text = f.read().decode("utf-8").lower()
        self.assertNotIn(user.lower(), text)
        self.assertNotIn(mn.ROOT.lower(), text)
        self.assertNotIn(mn.ROOT.replace("\\", "/").lower(), text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
