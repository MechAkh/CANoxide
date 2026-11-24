"""Write THIRD-PARTY-NOTICES.txt: the licences of the crates and fonts in the exes.

    py -3 tools\\make_notices.py [--check] [--out PATH] [--metadata FILE]

Runs "cargo metadata --format-version 1 --locked --filter-platform
x86_64-pc-windows-msvc" (no window), walks the resolve graph from the package
canoxide over the normal and build dependencies (not the dev-dependencies) and
writes, for each crate, its name, version, licence expression, repository and
the texts of the licence files in its package. Identical texts are printed
once. A last section names the fonts and icons that the programs embed.

The output is deterministic: LF line endings, crates sorted by name and
version, no date, no absolute path.

Options:
  --check          exit 1 when the file on disk differs from the generated text
  --out PATH       write (or check) PATH instead of THIRD-PARTY-NOTICES.txt at
                   the repository root
  --metadata FILE  read the cargo metadata JSON from FILE (no cargo call)

Exit code: 0 = written or up to date, 1 = --check found a difference,
2 = error. Standard library only.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_OUT = os.path.join(ROOT, "THIRD-PARTY-NOTICES.txt")
ROOT_PACKAGE = "canoxide"
TARGET = "x86_64-pc-windows-msvc"
LICENCE_PREFIXES = ("license", "licence", "copying", "notice", "unlicense")
FONT_EXTS = (".ttf", ".otf")
FONT_PACKAGES = ("egui", "epaint", "epaint_default_fonts")
# Files beside a font that can hold its licence text.
FONT_LICENCE_EXTS = ("", ".txt", ".md")
SKIP_DIRS = {"target", ".git"}
# The licences that need no closer look (the brief's list).
COMMON_LICENCES = {
    "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "Zlib", "ISC",
    "Unicode-3.0", "Unicode-DFS-2016", "BSL-1.0", "OFL-1.1", "CC0-1.0",
    "MPL-2.0", "Unlicense",
}
COMMON_EXCEPTIONS = {"LLVM-exception"}
RULE = "-" * 78
DRULE = "=" * 78


# ---------------------------------------------------------------- metadata

def find_cargo():
    cargo = shutil.which("cargo")
    if cargo:
        return cargo
    cand = os.path.join(os.path.expanduser("~"), ".cargo", "bin", "cargo.exe")
    if os.path.isfile(cand):
        return cand
    raise RuntimeError("cargo not found on PATH or in ~/.cargo/bin")


def run_cargo_metadata(root=ROOT):
    cmd = [find_cargo(), "metadata", "--format-version", "1", "--locked",
           "--filter-platform", TARGET,
           "--manifest-path", os.path.join(root, "Cargo.toml")]
    flags = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    p = subprocess.run(cmd, cwd=root, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                       stderr=subprocess.PIPE, creationflags=flags)
    if p.returncode != 0:
        raise RuntimeError("cargo metadata failed (exit %d):\n%s"
                           % (p.returncode, p.stderr.decode("utf-8", "replace")))
    return json.loads(p.stdout.decode("utf-8"))


def load_metadata(path=None):
    if path:
        with open(path, "r", encoding="utf-8") as f:
            return json.load(f)
    return run_cargo_metadata()


def root_package(meta):
    pkgs = {p["id"]: p for p in meta["packages"]}
    members = meta.get("workspace_members") or []
    for pid in members:
        if pid in pkgs and pkgs[pid]["name"] == ROOT_PACKAGE:
            return pkgs[pid]
    rid = (meta.get("resolve") or {}).get("root")
    if rid in pkgs and pkgs[rid]["name"] == ROOT_PACKAGE:
        return pkgs[rid]
    raise RuntimeError("package %r not found in the metadata" % ROOT_PACKAGE)


def _follows(dep):
    """True when a resolve edge is a normal or a build dependency."""
    kinds = dep.get("dep_kinds")
    if kinds is None:  # very old cargo: no kind information
        return True
    return any(k.get("kind") in (None, "normal", "build") for k in kinds)


def walk(meta):
    """The packages reached from the root over normal and build edges (root excluded)."""
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = root_package(meta)
    seen = {root["id"]}
    todo = [root["id"]]
    while todo:
        nid = todo.pop()
        node = nodes.get(nid)
        if node is None:
            continue
        for dep in node.get("deps", []):
            if _follows(dep) and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                todo.append(dep["pkg"])
    seen.discard(root["id"])
    return root, sorted((pkgs[i] for i in seen), key=crate_key)


def version_key(v):
    core, _, rest = v.partition("+")[0].partition("-")
    nums = []
    for part in core.split("."):
        nums.append(int(part) if part.isdigit() else -1)
    # A release sorts after its pre-releases.
    return (tuple(nums), 0 if rest else 1, rest, v)


def crate_key(p):
    return (p["name"], version_key(p["version"]), p["id"])


# ---------------------------------------------------------------- texts

def normalise(text):
    text = text.lstrip("﻿").replace("\r\n", "\n").replace("\r", "\n")
    lines = [ln.rstrip() for ln in text.split("\n")]
    while lines and not lines[0]:
        lines.pop(0)
    while lines and not lines[-1]:
        lines.pop()
    return "\n".join(lines)


def read_text(path):
    with open(path, "r", encoding="utf-8", errors="replace", newline="") as f:
        return normalise(f.read())


def rel(path, base):
    return os.path.relpath(path, base).replace("\\", "/")


def licence_files(pkg):
    """(relative name, absolute path) of each licence file of a package, sorted."""
    folder = os.path.dirname(pkg["manifest_path"])
    found = {}
    try:
        names = sorted(os.listdir(folder))
    except OSError:
        names = []
    for name in names:
        full = os.path.join(folder, name)
        if not name.lower().startswith(LICENCE_PREFIXES):
            continue
        if os.path.isfile(full):
            found[os.path.normcase(os.path.abspath(full))] = (name, full)
        elif os.path.isdir(full):
            # A REUSE-style LICENSES/ folder: take the files directly in it.
            for sub in sorted(os.listdir(full)):
                sfull = os.path.join(full, sub)
                if os.path.isfile(sfull):
                    found[os.path.normcase(os.path.abspath(sfull))] = (name + "/" + sub, sfull)
    lf = pkg.get("license_file")
    if lf:
        full = lf if os.path.isabs(lf) else os.path.join(folder, lf)
        if os.path.isfile(full):
            key = os.path.normcase(os.path.abspath(full))
            if key not in found:
                found[key] = (rel(full, folder), full)
    return sorted(found.values())


def licence_ids(expr):
    """The licence identifiers of an SPDX expression (also the old "A/B" form)."""
    if not expr:
        return []
    tokens = re.findall(r"[A-Za-z0-9.+\-]+", expr.replace("/", " OR "))
    ids, after_with = [], False
    for t in tokens:
        if t in ("AND", "OR"):
            after_with = False
            continue
        if t == "WITH":
            after_with = True
            continue
        ids.append(("exception", t) if after_with else ("licence", t))
        after_with = False
    return ids


def uncommon(expr):
    """True when the expression is empty or names a licence outside COMMON_LICENCES."""
    ids = licence_ids(expr)
    if not ids:
        return True
    for kind, t in ids:
        if kind == "licence" and t not in COMMON_LICENCES:
            return True
        if kind == "exception" and t not in COMMON_EXCEPTIONS:
            return True
    return False


# ---------------------------------------------------------------- fonts

def font_folders(pkg):
    """{relative folder: (fonts, licence files)} of a package, sorted."""
    base = os.path.dirname(pkg["manifest_path"])
    out = {}
    for dirpath, dirnames, filenames in os.walk(base):
        dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS)
        fonts = sorted(f for f in filenames if f.lower().endswith(FONT_EXTS))
        if not fonts:
            continue
        lic = sorted(f for f in filenames
                     if not f.lower().endswith(FONT_EXTS)
                     and os.path.splitext(f)[1].lower() in FONT_LICENCE_EXTS
                     and os.path.isfile(os.path.join(dirpath, f)))
        r = rel(dirpath, base)
        out["" if r == "." else r] = (fonts, lic, dirpath)
    return dict(sorted(out.items()))


# ---------------------------------------------------------------- render

def table(rows, header):
    widths = [max(len(r[i]) for r in rows + [header]) for i in range(len(header))]
    def line(r):
        return "  ".join(c.ljust(w) for c, w in zip(r, widths)).rstrip()
    out = [line(header), line(["-" * w for w in widths])]
    out += [line(r) for r in rows]
    return out


def collect(meta):
    root, crates = walk(meta)
    groups = {}       # text -> list of (crate key, label)
    no_file = []
    for p in crates:
        files = licence_files(p)
        if not files:
            no_file.append(p)
        for name, full in files:
            text = read_text(full)
            if not text:
                continue
            groups.setdefault(text, []).append(
                (crate_key(p), "%s %s (%s)" % (p["name"], p["version"], name)))
    ordered = sorted(groups.items(), key=lambda kv: (min(k for k, _ in kv[1]), kv[0]))
    return root, crates, ordered, no_file


def render(meta):
    root, crates, groups, no_file = collect(meta)
    by_name = {}
    for p in crates:
        by_name.setdefault(p["name"], []).append(p)
    o = []
    title = "THIRD-PARTY NOTICES FOR CANoxide %s" % root["version"]
    o += [title, "=" * len(title), ""]
    o += [
        "CANoxide.exe and canoxide-cli.exe contain code from the Rust crates that",
        "this file lists, and embed the fonts and icons of section 4. These are the",
        "normal and build dependencies of the package canoxide for the target",
        TARGET + " as Cargo.lock pins them (dev-dependencies, which only",
        "the tests use, are left out). The licence texts that the crate packages",
        "ship follow in section 2.",
        "",
        "This file is generated by tools/make_notices.py. Do not edit it by hand.",
        "",
        "Crates: %d" % len(crates),
        "Distinct licence texts: %d" % len(groups),
        "",
        "Sections:",
        "  1. Summary",
        "  2. Licence texts",
        "  3. No licence file in the package",
        "  4. Fonts and icons",
        "",
    ]

    o += [DRULE, "1. SUMMARY", DRULE, ""]
    rows = [[p["name"], p["version"], p.get("license") or "(none)"] for p in crates]
    o += table(rows, ["Crate", "Version", "Licence expression"])
    o += [""]

    o += [DRULE, "2. LICENCE TEXTS", DRULE, ""]
    o += ["Each text is printed once. The crates (and the file name in each crate",
          "package) that ship the text are listed above it.", ""]
    for i, (text, users) in enumerate(groups, 1):
        o += [RULE, "Text %d of %d. Used by:" % (i, len(groups))]
        o += ["  " + label for _, label in sorted(users)]
        o += [RULE, "", text, ""]

    o += [DRULE, "3. NO LICENCE FILE IN THE PACKAGE", DRULE, ""]
    if no_file:
        o += ["The packages of these crates hold no licence file. Their licence is the",
              "expression of their manifest. The standard text of each licence named",
              "there (for example MIT or Apache-2.0) is printed in section 2 for other",
              "crates under the same licence. The copyright holders of these crates",
              "are their authors (see the repository of each crate).", ""]
        rows = [[p["name"], p["version"], p.get("license") or "(none)",
                 p.get("repository") or "(none)"] for p in no_file]
        o += table(rows, ["Crate", "Version", "Licence expression", "Repository"])
    else:
        o += ["Every crate package holds a licence file."]
    o += [""]

    o += [DRULE, "4. FONTS AND ICONS", DRULE, ""]
    o += [
        "Inter (assets/fonts/Inter-Regular.ttf) and JetBrains Mono",
        "(assets/fonts/JetBrainsMono-Regular.ttf): SIL Open Font License 1.1. The",
        "texts are licenses/Inter-OFL.txt and licenses/JetBrainsMono-OFL.txt in the",
        "install folder.",
        "",
    ]
    ph = by_name.get("egui-phosphor", [])
    if ph:
        for p in ph:
            o += ["Phosphor icons: the Phosphor icon font, through the crate egui-phosphor",
                  "%s (crate licence: %s). The README of that crate states that the"
                  % (p["version"], p.get("license") or "(none)"),
                  "Phosphor Icons are licensed under MIT. The licence texts of the crate are",
                  "in section 2.", ""]
    else:
        o += ["Phosphor icons: the crate egui-phosphor is not a dependency.", ""]

    font_pkgs = [p for p in crates if p["name"] in FONT_PACKAGES]
    found = [(p, font_folders(p)) for p in font_pkgs]
    found = [(p, ff) for p, ff in found if ff]
    o += ["Default fonts of egui: the font files (*.ttf, *.otf) in the packages",
          "%s, and the licence files that lie beside them." % ", ".join(FONT_PACKAGES), ""]
    if not found:
        o += ["  No font file was found in these packages.", ""]
    texts = []
    for p, ff in found:
        o += ["Package %s %s (licence expression: %s):"
              % (p["name"], p["version"], p.get("license") or "(none)")]
        for folder, (fonts, lic, full) in ff.items():
            o += ["  Folder %s/" % folder if folder else "  Package root folder"]
            for f in fonts:
                stem = os.path.splitext(f)[0].lower()
                match = [x for x in lic if x.lower().startswith(stem)]
                note = "  (licence file by name: %s)" % ", ".join(match) if match else ""
                o += ["    font     %s%s" % (f, note)]
            for f in lic:
                o += ["    licence  %s" % f]
                texts.append(("%s %s: %s" % (p["name"], p["version"],
                                             (folder + "/" if folder else "") + f),
                              read_text(os.path.join(full, f))))
            if not lic:
                o += ["    (no licence file beside the fonts)"]
        o += [""]
    for label, text in texts:
        o += [RULE, label, RULE, "", text, ""]

    out = "\n".join(o).rstrip("\n") + "\n"
    check_no_paths(out, meta)
    return out


def check_no_paths(out, meta):
    """Refuse an output that names a local folder (absolute paths, user name)."""
    low = out.lower()
    dirs = {os.path.expanduser("~"), meta.get("workspace_root") or ""}
    for p in meta["packages"]:
        dirs.add(os.path.dirname(p.get("manifest_path") or ""))
    for d in sorted(dirs):
        if len(d) < 4:
            continue
        for v in (d, d.replace("\\", "/"), d.replace("/", "\\")):
            if v.lower() in low:
                raise RuntimeError("the output holds a local path: %s" % v)


# ---------------------------------------------------------------- main

def summary(meta, out_bytes):
    root, crates, groups, no_file = collect(meta)
    counts = {}
    for p in crates:
        e = p.get("license") or "(none)"
        counts[e] = counts.get(e, 0) + 1
    lines = ["crates: %d, distinct licence texts: %d, without licence file: %d"
             % (len(crates), len(groups), len(no_file)),
             "size: %d bytes" % len(out_bytes), "licence expressions:"]
    for e, n in sorted(counts.items(), key=lambda kv: (-kv[1], kv[0])):
        lines.append("  %4d  %s" % (n, e))
    odd = [p for p in crates if uncommon(p.get("license"))]
    lines.append("outside the common list: %d" % len(odd))
    for p in odd:
        lines.append("  %s %s: %s" % (p["name"], p["version"], p.get("license") or "(none)"))
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description="Write THIRD-PARTY-NOTICES.txt")
    ap.add_argument("--check", action="store_true",
                    help="exit 1 when the file on disk differs from the generated text")
    ap.add_argument("--out", default=DEFAULT_OUT, help="output path")
    ap.add_argument("--metadata", help="cargo metadata JSON file (no cargo call)")
    ap.add_argument("--quiet", action="store_true", help="no summary on stdout")
    a = ap.parse_args(argv)
    try:
        meta = load_metadata(a.metadata)
        data = render(meta).encode("utf-8")
    except (RuntimeError, OSError, ValueError, KeyError) as e:
        print("error: %s" % e, file=sys.stderr)
        return 2
    if a.check:
        try:
            with open(a.out, "rb") as f:
                have = f.read()
        except OSError:
            have = None
        if have != data:
            print("%s is %s; run py -3 tools\\make_notices.py"
                  % (os.path.basename(a.out), "missing" if have is None else "out of date"),
                  file=sys.stderr)
            return 1
        if not a.quiet:
            print("%s is up to date" % os.path.basename(a.out))
        return 0
    with open(a.out, "wb") as f:
        f.write(data)
    if not a.quiet:
        print("wrote %s" % os.path.basename(a.out))
        print(summary(meta, data))
    return 0


if __name__ == "__main__":
    sys.exit(main())
