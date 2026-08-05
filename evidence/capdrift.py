#!/usr/bin/env python3
"""Capability drift across real npm releases.

rigid records an imported definition's capability set when it pins it, and
reports a widening (W0405) as a build failure under --frozen. Whether that is
useful is an empirical question, not a design one: if packages gain
capabilities constantly, the signal is noise; if a patch release almost never
starts touching the network, then the one that does is worth stopping.

So this measures it. For each package, consecutive published versions are
downloaded and scanned for the host capabilities their code can reach. The
delta between neighbours is classified by the semver step between them.

What this is NOT: a JavaScript analysis as exact as rigid's own. rigid knows
its capabilities because they are *declared* on `extern` and propagated along
a graph the compiler built. Here they are recovered by scanning for the
require/global forms that reach each capability, which over-approximates
(a string "child_process" in a comment counts) and under-approximates
(fully dynamic require(x) does not). It is a proxy for what the declarations
would say, and the number should be read as such.

Usage:
    python3 evidence/capdrift.py --packages 60 --versions 12
    python3 evidence/capdrift.py --report        # re-render from the cache
"""

import argparse
import io
import json
import os
import re
import sys
import tarfile
import urllib.error
import urllib.request
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.join(HERE, "capdrift-cache.json")
OUT = os.path.join(HERE, "capdrift-result.json")

# The capability vocabulary, mapped onto what JavaScript uses to reach it.
# Node builtins are matched through require()/import of the bare or node:
# prefixed name; the browser/global forms are matched directly.
NODE_MODULES = {
    "net": ["http", "https", "net", "dgram", "dns", "tls", "http2"],
    "fs": ["fs", "fs/promises", "path"],
    "proc": ["child_process", "cluster", "worker_threads", "vm"],
    "env": ["os", "process"],
}
GLOBAL_FORMS = {
    "net": [r"\bfetch\s*\(", r"\bXMLHttpRequest\b", r"\bWebSocket\b", r"\bnavigator\.sendBeacon\b"],
    "proc": [r"\beval\s*\(", r"\bnew\s+Function\s*\("],
    "env": [r"\bprocess\.env\b"],
    "time": [r"\bDate\.now\s*\(", r"\bnew\s+Date\s*\("],
    "rand": [r"\bMath\.random\s*\(", r"\bcrypto\.getRandomValues\b", r"\brandomBytes\s*\("],
}
INSTALL_HOOKS = ("preinstall", "install", "postinstall")

# `path` is in the fs list because it is the fs namespace, but on its own it
# touches nothing. Kept separate so it never alone implies fs.
HARMLESS = {"path"}

REQUIRE_RE = re.compile(
    r"""(?:require\s*\(\s*|from\s+|import\s*\(\s*)['"](?:node:)?([a-z_/]+)['"]""", re.I
)


def http_json(url, timeout=30):
    req = urllib.request.Request(url, headers={"User-Agent": "rigid-evidence/1.0"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


# The sample is the dependency closure of packages people actually install,
# rather than a search ranking — npm's search API scores text relevance, so
# asking it for "popular packages" returns whatever matches the query word.
# These seeds were chosen before any result was seen.
SEEDS = [
    "react", "react-dom", "vue", "express", "webpack", "eslint", "jest",
    "typescript", "next", "axios", "lodash", "rollup", "vite", "prettier",
    "@babel/core", "mocha", "chai", "commander", "chalk", "socket.io",
    "mongoose", "redux", "svelte", "nodemon", "dotenv", "yargs", "got",
]


def top_packages(n):
    """Real, widely-installed packages: the seeds plus their dependencies."""
    seen, order = set(), []

    def add(nm):
        if nm not in seen and not nm.startswith("@types/"):
            seen.add(nm)
            order.append(nm)

    for s in SEEDS:
        add(s)
    frontier = list(SEEDS)
    while len(order) < n and frontier:
        nxt = []
        for pkg in frontier:
            if len(order) >= n:
                break
            try:
                d = http_json(f"https://registry.npmjs.org/{urllib.parse.quote(pkg, safe='@')}")
                latest = d.get("dist-tags", {}).get("latest")
                deps = d.get("versions", {}).get(latest, {}).get("dependencies", {}) or {}
            except Exception:
                continue
            for dep in deps:
                add(dep)
                nxt.append(dep)
        frontier = nxt
    return order[:n]


def semver_step(a, b):
    """patch / minor / major / other, for the step from a to b."""
    def parts(v):
        core = re.split(r"[-+]", v)[0]
        bits = core.split(".")
        return [int(x) if x.isdigit() else -1 for x in (bits + ["0", "0"])[:3]]

    try:
        x, y = parts(a), parts(b)
    except Exception:
        return "other"
    if -1 in x or -1 in y:
        return "other"
    if y[0] != x[0]:
        return "major"
    if y[1] != x[1]:
        return "minor"
    if y[2] != x[2]:
        return "patch"
    return "other"


REL_RE = re.compile(
    r"""(?:require\s*\(\s*|from\s+|import\s*\(\s*)['"](\.[^'"]*)['"]"""
)


def _entry_points(pj):
    """Files a consumer actually loads: main, module, and the exports map."""
    out = []
    for k in ("main", "module", "browser", "unpkg"):
        v = pj.get(k)
        if isinstance(v, str):
            out.append(v)
    def walk(x):
        if isinstance(x, str):
            out.append(x)
        elif isinstance(x, dict):
            for v in x.values():
                walk(v)
        elif isinstance(x, list):
            for v in x:
                walk(v)
    walk(pj.get("exports"))
    return [o for o in out if isinstance(o, str)]


def _norm(path):
    parts = []
    for seg in path.replace("\\", "/").split("/"):
        if seg in ("", "."):
            continue
        if seg == "..":
            if parts:
                parts.pop()
        else:
            parts.append(seg)
    return "/".join(parts)


def _resolve(base_dir, spec, files):
    """Node-ish resolution of a relative specifier against the tarball."""
    cand = _norm(f"{base_dir}/{spec}") if base_dir else _norm(spec)
    for c in (cand, cand + ".js", cand + ".mjs", cand + ".cjs",
              cand + "/index.js", cand + "/index.mjs", cand + "/index.cjs"):
        if c in files:
            return c
    return None


def scan_tarball(data):
    """The capability set a package's *loadable* code can reach.

    Only files reachable from the package's declared entry points count.
    A tarball also ships release tooling — lodash 4.17.14 has a `tag.js` that
    requires child_process — and counting those would report a capability the
    consumer never loads. Install hooks are the exception: they run whether
    anything imports them or not.

    A truncated or malformed tarball yields (None, None) rather than raising:
    one unreadable artifact must not end a run over thousands of them.
    """
    caps = set()
    hooks = []
    try:
        tf = tarfile.open(fileobj=io.BytesIO(data), mode="r:gz")
        members = tf.getmembers()
    except Exception:
        return None, None

    files = {}
    pj = {}
    for m in members:
        if not m.isfile() or m.size > 4_000_000:
            continue
        rel = m.name.split("/", 1)[1] if "/" in m.name else m.name
        if rel == "package.json":
            try:
                pj = json.loads(tf.extractfile(m).read().decode("utf-8", "replace"))
            except Exception:
                pj = {}
            continue
        if rel.lower().endswith((".js", ".mjs", ".cjs")):
            files[rel] = m

    for h in INSTALL_HOOKS:
        if isinstance(pj.get("scripts"), dict) and h in pj["scripts"]:
            hooks.append(h)

    entries = [e for e in (_norm(x) for x in _entry_points(pj)) if e]
    roots = [e for e in (_resolve("", e, files) for e in entries) if e]
    if not roots:
        for guess in ("index.js", "index.mjs", "index.cjs", "dist/index.js", "lib/index.js"):
            if guess in files:
                roots = [guess]
                break
    if not roots:
        # Nothing declares an entry and no conventional one exists: fall back
        # to every file, and accept the over-count rather than report nothing.
        roots = list(files)

    seen = set()
    stack = list(roots)
    while stack:
        cur = stack.pop()
        if cur in seen or cur not in files:
            continue
        seen.add(cur)
        try:
            src = tf.extractfile(files[cur]).read().decode("utf-8", "replace")
        except Exception:
            continue
        for mod in REQUIRE_RE.findall(src):
            mod = mod.lower()
            if mod in HARMLESS:
                continue
            for cap, mods in NODE_MODULES.items():
                if mod in mods:
                    caps.add(cap)
        for cap, pats in GLOBAL_FORMS.items():
            if cap in caps:
                continue
            for p in pats:
                if re.search(p, src):
                    caps.add(cap)
                    break
        base = cur.rsplit("/", 1)[0] if "/" in cur else ""
        for spec in REL_RE.findall(src):
            nxt = _resolve(base, spec, files)
            if nxt:
                stack.append(nxt)

    if hooks:
        caps.add("install-hook")
    return sorted(caps), sorted(set(hooks))


def safe_scan(data):
    try:
        return scan_tarball(data)
    except Exception:
        return None, None


def fetch_versions(pkg, k):
    """The last k released versions, oldest first, with their tarball URLs."""
    try:
        d = http_json(f"https://registry.npmjs.org/{urllib.parse.quote(pkg, safe='@')}")
    except Exception:
        return []
    vs = d.get("versions", {})
    times = d.get("time", {})
    ordered = [v for v in vs if v in times and not re.search(r"[-+]", v)]
    ordered.sort(key=lambda v: times[v])
    picked = ordered[-k:]
    return [(v, vs[v].get("dist", {}).get("tarball")) for v in picked if vs[v].get("dist")]


def one_package(pkg, k):
    try:
        return _one_package(pkg, k)
    except Exception:
        return pkg, []


def _one_package(pkg, k):
    out = []
    for v, url in fetch_versions(pkg, k):
        if not url:
            continue
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "rigid-evidence/1.0"})
            with urllib.request.urlopen(req, timeout=60) as r:
                data = r.read(12_000_000)
        except Exception:
            continue
        caps, hooks = safe_scan(data)
        if caps is None:
            continue
        out.append({"version": v, "caps": caps, "hooks": hooks})
    return pkg, out


def analyse(cache):
    steps = Counter()
    widened = Counter()
    gains = defaultdict(Counter)
    examples = defaultdict(list)
    pkgs_with_widening = set()

    for pkg, versions in cache.items():
        for i in range(1, len(versions)):
            a, b = versions[i - 1], versions[i]
            step = semver_step(a["version"], b["version"])
            if step == "other":
                continue
            steps[step] += 1
            new = set(b["caps"]) - set(a["caps"])
            if new:
                widened[step] += 1
                pkgs_with_widening.add(pkg)
                for c in new:
                    gains[step][c] += 1
                if len(examples[step]) < 12:
                    examples[step].append(
                        {
                            "package": pkg,
                            "from": a["version"],
                            "to": b["version"],
                            "gained": sorted(new),
                        }
                    )
    return steps, widened, gains, examples, pkgs_with_widening


def report(cache):
    steps, widened, gains, examples, pkgs = analyse(cache)
    total = sum(steps.values())
    nver = sum(len(v) for v in cache.values())
    lines = []
    add = lines.append
    add("Capability drift across real npm releases")
    add("=" * 58)
    add(f"{len(cache)} packages, {nver} versions, {total} consecutive upgrades examined")
    add("")
    add(f"{'step':<8}{'upgrades':>10}{'widened':>10}{'rate':>9}")
    for s in ("patch", "minor", "major"):
        n, w = steps.get(s, 0), widened.get(s, 0)
        rate = f"{100.0 * w / n:.1f}%" if n else "—"
        add(f"{s:<8}{n:>10}{w:>10}{rate:>9}")
    add("")
    add(f"{len(pkgs)} of {len(cache)} packages widened at least once.")
    add("")
    for s in ("patch", "minor", "major"):
        if not gains[s]:
            continue
        top = ", ".join(f"{c} x{n}" for c, n in gains[s].most_common(8))
        add(f"capabilities gained on a {s}: {top}")
    add("")
    add("Sample of patch-level widenings (the kind a lockfile lets through):")
    for e in examples.get("patch", [])[:10]:
        add(f"  {e['package']} {e['from']} -> {e['to']}   +{' +'.join(e['gained'])}")
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--packages", type=int, default=60)
    ap.add_argument("--versions", type=int, default=12)
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--report", action="store_true")
    a = ap.parse_args()

    if a.report:
        # Never silently fall through to a fetch: that would overwrite the
        # cache with a different sample and quietly change the population.
        if not os.path.exists(CACHE):
            sys.exit(f"no cache at {CACHE} — run without --report first")
        print(report(json.load(open(CACHE))))
        return

    names = top_packages(a.packages)
    print(f"fetching {len(names)} packages x up to {a.versions} versions", file=sys.stderr)
    json.dump(names, open(os.path.join(HERE, "capdrift-sample.json"), "w"), indent=1)
    cache = {}
    if os.path.exists(CACHE):
        cache = json.load(open(CACHE))
    # A cache from a different sample would silently mix populations.
    cache = {k: v for k, v in cache.items() if k in set(names)}
    todo = [n for n in names if n not in cache]
    with ThreadPoolExecutor(max_workers=a.workers) as ex:
        for i, (pkg, vs) in enumerate(ex.map(lambda p: one_package(p, a.versions), todo)):
            if vs:
                cache[pkg] = vs
            if (i + 1) % 10 == 0:
                print(f"  {i + 1}/{len(todo)}", file=sys.stderr)
                json.dump(cache, open(CACHE, "w"))
    json.dump(cache, open(CACHE, "w"))

    steps, widened, gains, examples, pkgs = analyse(cache)
    json.dump(
        {
            "packages": len(cache),
            "versions": sum(len(v) for v in cache.values()),
            "steps": dict(steps),
            "widened": dict(widened),
            "gains": {k: dict(v) for k, v in gains.items()},
            "examples": {k: v for k, v in examples.items()},
        },
        open(OUT, "w"),
        indent=1,
    )
    print(report(cache))


if __name__ == "__main__":
    import urllib.parse

    main()
