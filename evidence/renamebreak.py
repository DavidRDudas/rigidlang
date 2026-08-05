#!/usr/bin/env python3
"""How often does a rename leave a dangling reference in real code?

rigid's central claim is that a name-bound language lets a rename silently
break a reference, and that content-addressed identity turns that into a
compile error (E0201, with a mechanical fix). The claim has been argued from
first principles and measured on a benchmark written by the same author.
Neither is evidence about real code.

This measures it on real code, in a language rigid cannot compile.

Method. Walk a repository's history commit by commit. At each commit, parse
every changed Python file with Python's own `ast` and record its top-level
`def`/`class` names. A name that disappears from a file while a same-arity
name appears in the same file is treated as a **rename** rather than a
delete-plus-add. Then ask the question that matters: immediately after that
commit, does any *other* file in the tree still reference the old name?

If it does, the rename shipped an incomplete edit. That is exactly the state
rigid refuses to produce: the pinned identity is found under the new name and
the reference is reported with a `rename_reference` fix.

Honest limits, because the number is worth less without them:

  * Python is dynamic. A surviving textual reference is not proof of a
    runtime break — it may be shadowed, guarded, in dead code, or a
    coincidental match on a common word. Matches are therefore required to
    be call-shaped or import-shaped, and single-character and dunder names
    are dropped. This still over-counts.
  * Renames are inferred from the shape of a diff, not from intent. A
    delete and an unrelated add in one commit can look like a rename.
  * Only the repository's own files are searched. A break in a *dependent*
    project is invisible here, and that is the larger population.

The output is therefore a rate of *plausible* dangling references, not of
confirmed production incidents. It is still a number about somebody else's
code, which is the point.

Usage:
    python3 evidence/renamebreak.py --repo https://github.com/psf/requests
    python3 evidence/renamebreak.py --repos evidence/repos.txt --commits 3000
"""

import argparse
import ast
import json
import os
import re
import subprocess
import sys
import tempfile
import warnings
from collections import Counter

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "renamebreak-result.json")

SKIP_NAMES = {"main", "run", "test", "setup", "get", "set", "add", "remove", "update"}

# An added or removed line that declares a module-level def or class.
DEFLINE_RE = re.compile(r"^[+-](?:async +)?(?:def|class) +\w", re.M)


def git(repo, *args, **kw):
    return subprocess.run(
        ["git", "-C", repo, *args],
        capture_output=True,
        text=True,
        errors="replace",
        **kw,
    ).stdout


def top_level_defs(src):
    """name -> (kind, arity) for module-level defs and classes.

    Old revisions of these projects contain Python 2 and pre-PEP-8 escapes
    that parse with warnings; they are not interesting here.
    """
    try:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            tree = ast.parse(src)
    except Exception:
        return None
    out = {}
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            out[node.name] = ("def", len(node.args.args))
        elif isinstance(node, ast.ClassDef):
            out[node.name] = ("class", 0)
    return out


def file_at(repo, rev, path):
    r = subprocess.run(
        ["git", "-C", repo, "show", f"{rev}:{path}"],
        capture_output=True,
        text=True,
        errors="replace",
    )
    return r.stdout if r.returncode == 0 else None


def referencing_files(repo, rev, name, exclude):
    """Files at `rev` that use `name` in a call- or import-shaped position.

    `-P` (PCRE), not `-E`: git's ERE engine does not implement `\b`, and it
    fails by matching *nothing* rather than by complaining — which silently
    turns this whole study into a machine that always answers zero. The
    controls in `--selftest` exist because that happened.

    Three shapes count: a bare call, a qualified call that is not on `self`
    or `cls`, and an import naming it.
    """
    n = re.escape(name)
    pat = (
        rf"(?<![\w.]){n}\s*\("
        rf"|(?<!self)(?<!cls)\.{n}\s*\("
        rf"|^[ \t]*(from|import)[ \t][^\n]*\b{n}\b"
    )
    r = subprocess.run(
        ["git", "-C", repo, "grep", "-l", "-P", pat, rev, "--", "*.py"],
        capture_output=True,
        text=True,
        errors="replace",
    )
    hits = []
    for line in r.stdout.splitlines():
        if ":" not in line:
            continue
        path = line.split(":", 1)[1]
        if path != exclude:
            hits.append(path)
    return hits


def still_defined(repo, rev, name):
    """Is `name` still defined as a top-level def/class anywhere at `rev`?

    If it is, the disappearance from one file was a move, a consolidation, or
    a second definition of a common name — not a rename. Without this check
    `batched` in more-itertools reads as renamed to `constrained_batches`
    while `batched` is still a public function, and every `test_it` in pytest
    reads as renamed to whatever else that commit added.
    """
    n = re.escape(name)
    r = subprocess.run(
        ["git", "-C", repo, "grep", "-l", "-P",
         rf"^[ \t]*(async +)?(def|class) +{n}\b", rev, "--", "*.py"],
        capture_output=True, text=True, errors="replace",
    )
    return bool(r.stdout.strip())


def analyse_repo(url, max_commits, workdir):
    name = url.rstrip("/").split("/")[-1].replace(".git", "")
    repo = os.path.join(workdir, name)
    if not os.path.isdir(repo):
        print(f"  cloning {name}", file=sys.stderr)
        subprocess.run(
            ["git", "clone", "--quiet", "--filter=blob:none", url, repo],
            check=True,
            capture_output=True,
        )
    revs = git(repo, "rev-list", "--reverse", f"--max-count={max_commits}", "HEAD").split()
    print(f"  {name}: {len(revs)} commits", file=sys.stderr)

    renames = 0
    dangling = 0
    findings = []
    for i, rev in enumerate(revs):
        if i == 0:
            continue
        # A commit only matters if a top-level `def`/`class` line moved.
        # Checking that from the diff first avoids two `git show` calls per
        # changed file on the overwhelming majority of commits, which is the
        # difference between an hour per repository and a few minutes.
        d = git(repo, "diff", "--unified=0", f"{rev}~1", rev, "--", "*.py")
        if not DEFLINE_RE.search(d):
            continue
        changed = git(repo, "diff", "--name-only", f"{rev}~1", rev, "--", "*.py").split()
        if not changed or len(changed) > 40:
            continue
        for path in changed:
            before_src = file_at(repo, f"{rev}~1", path)
            after_src = file_at(repo, rev, path)
            if before_src is None or after_src is None:
                continue
            before = top_level_defs(before_src)
            after = top_level_defs(after_src)
            if before is None or after is None:
                continue
            gone = {k: v for k, v in before.items() if k not in after}
            fresh = {k: v for k, v in after.items() if k not in before}
            if not gone or not fresh:
                continue
            # Pair a disappearance with an appearance of the same kind and
            # arity: that shape is a rename far more often than it is an
            # unrelated delete and add in one commit.
            for old, sig in gone.items():
                if len(old) < 4 or old.startswith("__") or old in SKIP_NAMES:
                    continue
                cands = [n for n, s in fresh.items() if s == sig and n != old]
                if len(cands) != 1:
                    continue
                new = cands[0]
                # Only a rename if the old name is gone from the whole tree.
                if still_defined(repo, rev, old):
                    continue
                renames += 1
                left = referencing_files(repo, rev, old, path)
                if left:
                    dangling += 1
                    findings.append(
                        {
                            "repo": name,
                            "commit": rev[:12],
                            "file": path,
                            "old": old,
                            "new": new,
                            "still_referenced_in": left[:6],
                        }
                    )
    return {
        "repo": name,
        "url": url,
        "commits": len(revs),
        "renames": renames,
        "dangling": dangling,
        "findings": findings,
    }


def selftest(repo):
    """Prove the reference detector works before trusting a zero from it.

    The controls are derived from the repository itself rather than
    hardcoded, so this runs anywhere: take top-level names defined at HEAD,
    confirm a plain fixed-string search finds each used in another file, and
    require the real detector to find it too.
    """
    head = git(repo, "rev-parse", "HEAD").strip()
    paths = [p for p in git(repo, "ls-tree", "-r", "--name-only", head).split()
             if p.endswith(".py")][:400]
    candidates = []
    for path in paths:
        src = file_at(repo, head, path)
        if not src:
            continue
        for name in (top_level_defs(src) or {}):
            if len(name) > 6 and not name.startswith("_"):
                candidates.append((name, path))
        if len(candidates) > 200:
            break

    ok = False
    checked = 0
    for name, home in candidates:
        raw = subprocess.run(
            ["git", "-C", repo, "grep", "-l", "-F", f"{name}(", head, "--", "*.py"],
            capture_output=True, text=True, errors="replace",
        ).stdout.splitlines()
        elsewhere = [l.split(":", 1)[1] for l in raw if ":" in l]
        elsewhere = [p for p in elsewhere if p != home]
        if len(elsewhere) < 2:
            continue
        checked += 1
        hits = referencing_files(repo, head, name, home)
        print(f"  positive {name:<22} plain={len(elsewhere)} detector={len(hits)}")
        if hits:
            ok = True
        if checked >= 3:
            break

    miss = referencing_files(repo, head, "zzz_not_a_real_name", "none.py")
    print(f"  negative control{'':<24} detector={len(miss)}")
    if miss:
        ok = False
    if checked == 0:
        print("  (no cross-file control found; skipping)")
        return True
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--selftest", metavar="REPO_DIR")
    ap.add_argument("--repo", action="append", default=[])
    ap.add_argument("--repos", help="file with one repo URL per line")
    ap.add_argument("--commits", type=int, default=1500)
    ap.add_argument("--workdir", default=None)
    a = ap.parse_args()

    if a.selftest:
        sys.exit(0 if selftest(a.selftest) else "SELFTEST FAILED")

    urls = list(a.repo)
    if a.repos:
        urls += [l.strip() for l in open(a.repos) if l.strip() and not l.startswith("#")]
    if not urls:
        ap.error("give --repo or --repos")

    workdir = a.workdir or tempfile.mkdtemp(prefix="renamebreak-")
    print(f"workdir: {workdir}", file=sys.stderr)

    # Resume: a completed repository is never re-analysed, and the file is
    # rewritten after each one. The first run of this study was killed at
    # repository eight and lost everything, because it only wrote at the end.
    results = []
    done = set()
    if os.path.exists(OUT):
        try:
            prev = json.load(open(OUT))
            results = prev.get("repos", [])
            done = {r["url"] for r in results}
        except Exception:
            pass

    def save():
        json.dump(
            {
                "repos": results,
                "total_renames": sum(r["renames"] for r in results),
                "total_dangling": sum(r["dangling"] for r in results),
            },
            open(OUT, "w"),
            indent=1,
        )

    # Run the controls against the first repository before measuring
    # anything. A detector that silently matches nothing would otherwise
    # report a confident, meaningless zero.
    first = os.path.join(workdir, urls[0].rstrip("/").split("/")[-1].replace(".git", ""))
    if os.path.isdir(first):
        print("  self-test:", file=sys.stderr)
        if not selftest(first):
            sys.exit("SELFTEST FAILED — the reference detector matches nothing")

    for u in urls:
        if u in done:
            print(f"  {u.split('/')[-1]}: cached", file=sys.stderr)
            continue
        try:
            results.append(analyse_repo(u, a.commits, workdir))
            save()
        except Exception as e:
            print(f"  {u}: {e}", file=sys.stderr)

    save()
    tot_r = sum(r["renames"] for r in results)
    tot_d = sum(r["dangling"] for r in results)

    print()
    print("Renames that shipped a dangling reference")
    print("=" * 62)
    print(f"{'repository':<22}{'commits':>9}{'renames':>9}{'dangling':>10}{'rate':>8}")
    for r in results:
        rate = f"{100.0 * r['dangling'] / r['renames']:.1f}%" if r["renames"] else "—"
        print(f"{r['repo']:<22}{r['commits']:>9}{r['renames']:>9}{r['dangling']:>10}{rate:>8}")
    rate = f"{100.0 * tot_d / tot_r:.1f}%" if tot_r else "—"
    print(f"{'TOTAL':<22}{'':>9}{tot_r:>9}{tot_d:>10}{rate:>8}")
    print()
    print("Examples:")
    for r in results:
        for f in r["findings"][:3]:
            print(f"  {f['repo']} {f['commit']}  {f['old']} -> {f['new']}")
            print(f"      still referenced in {', '.join(f['still_referenced_in'][:3])}")


if __name__ == "__main__":
    main()
