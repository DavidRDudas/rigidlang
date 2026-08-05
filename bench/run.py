#!/usr/bin/env python3
"""Breakage-detection benchmark: TypeScript's compiler vs `rigid check`.

The same program is written twice — once in strict TypeScript, once in rigid —
and the same logical breakage is applied to both. Each toolchain then gets to
report it. Nothing here is scored by hand: a scenario is CAUGHT if the checker
exits non-zero, and the first diagnostic it printed is recorded verbatim.

    python3 bench/run.py            # table
    python3 bench/run.py --json     # machine-readable

Both checkers run in their strongest CI configuration: `tsc --strict --noEmit`
and `rigid check`. Neither is given a hint about what changed.
"""

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
TSC = ROOT / "node_modules" / ".bin" / "tsc"
RIGID = REPO / "target" / "release" / "rigid"


def sub(path, old, new, count=0):
    """A patch step: replace `old` with `new` in `path`, asserting it matched."""
    return ("sub", path, old, new, count)


def append(path, text):
    return ("append", path, text, None, 0)


# Each scenario applies the same logical change to both programs.
SCENARIOS = [
    {
        "id": "rename-miss",
        "what": "Rename a definition, leave its callers behind",
        "why": "The everyday case. Both toolchains should catch it.",
        "ts": [sub("core.ts", "export function formatIncident", "export function renderIncident")],
        "rg": [sub("core.rg", "fn format_incident(inc) {", "fn render_incident(inc) {")],
    },
    {
        "id": "rename-reuse",
        "what": "Rename a definition, then give a DIFFERENT function its old name",
        "why": "Callers still compile, but now mean something else. This is the case "
               "rigid exists for.",
        "ts": [
            sub("core.ts", "export function notifyOncall", "export function pageOncall"),
            sub("core.ts", 'return "ESCALATE " + notifyOncall(inc);',
                'return "ESCALATE " + pageOncall(inc);'),
            append("core.ts",
                   "\n// Added later by someone else: same shape, different meaning.\n"
                   "export function notifyOncall(inc: Incident): string {\n"
                   '  return "SILENT " + formatIncident(inc);\n'
                   "}\n"),
        ],
        "rg": [
            sub("core.rg", "fn notify_oncall(inc) {", "fn page_oncall(inc) {"),
            sub("core.rg", 'return "ESCALATE " + notify_oncall(inc);',
                'return "ESCALATE " + page_oncall(inc);'),
            append("core.rg",
                   "\n// Added later by someone else: same shape, different meaning.\n"
                   "fn notify_oncall(inc) {\n"
                   '  return "SILENT " + format_incident(inc);\n'
                   "}\n"),
        ],
    },
    {
        "id": "arity-change",
        "what": "Add a parameter to a function, miss one call site",
        "why": "Arity is the one type-like check rigid has.",
        "ts": [
            sub("core.ts", "export function severity(impact: number, urgency: number): number {",
                "export function severity(impact: number, urgency: number, weight: number): number {"),
            sub("core.ts", "return impact * urgency;", "return impact * urgency * weight;"),
        ],
        "rg": [
            sub("core.rg", "fn severity(impact, urgency) {", "fn severity(impact, urgency, weight) {"),
            sub("core.rg", "return impact * urgency;", "return impact * urgency * weight;"),
        ],
    },
    {
        "id": "delete-referenced",
        "what": "Delete a function the dispatch set still points at",
        "why": "A reference to something that no longer exists.",
        "ts": [sub("core.ts",
                   'export function queueNormal(inc: Incident): string {\n'
                   '  return "QUEUE " + formatIncident(inc);\n}\n', "")],
        "rg": [sub("core.rg",
                   'fn queue_normal(inc) {\n  return "QUEUE " + format_incident(inc);\n}\n', "")],
    },
    {
        "id": "type-mismatch",
        "what": "Pass a string where a number belongs",
        "why": "rigid has no types beyond arity. TypeScript should win this one.",
        "ts": [sub("router.ts", "return severity(5, 5);", 'return severity("high", 5);')],
        "rg": [sub("router.rg", "return severity(5, 5);", 'return severity("high", 5);')],
    },
    {
        "id": "type-via-local",
        "what": "Bind a string to a local, then pass it where a number belongs",
        "why": "One indirection past a literal.",
        "ts": [sub("router.ts", "return severity(5, 5);",
                   'const k = "high";\n  return severity(k, 5);')],
        "rg": [sub("router.rg", "return severity(5, 5);",
                   'let k = "high";\n  return severity(k, 5);')],
    },
    {
        "id": "type-via-return",
        "what": "Pass a function's string result where a number belongs",
        "why": "Requires knowing what the callee returns.",
        "ts": [sub("router.ts", "return severity(5, 5);",
                   'return severity(formatIncident({ kind: "x", impact: 1, urgency: 1 }), 5);'),
               sub("router.ts", "import { Handler, notifyOncall, queueNormal, escalate, severity } from \"./core.js\";",
                   "import { Handler, notifyOncall, queueNormal, escalate, severity, formatIncident } from \"./core.js\";")],
        "rg": [sub("router.rg", "return severity(5, 5);",
                   'return severity(format_incident({ kind: "x", impact: 1, urgency: 1 }), 5);'),
               sub("router.rg", "import { notify_oncall, queue_normal, escalate, severity } from \"./core.rg\";",
                   "import { notify_oncall, queue_normal, escalate, severity, format_incident } from \"./core.rg\";")],
    },
    {
        "id": "arith-on-string",
        "what": "Multiply a string",
        "why": "Wrong on its face, with no call involved.",
        "ts": [sub("router.ts", "return severity(5, 5);", 'return "abc" * 2;')],
        "rg": [sub("router.rg", "return severity(5, 5);", 'return "abc" * 2;')],
    },
    {
        "id": "field-on-int",
        "what": "Read a field off an integer",
        "why": "Records have fields; integers do not.",
        "ts": [sub("router.ts", "return severity(5, 5);",
                   "const n = 5;\n  return n.kind;")],
        "rg": [sub("router.rg", "return severity(5, 5);", "let n = 5;\n  return n.kind;")],
    },
    {
        "id": "table-target-missing",
        "what": "Point a dispatch key at a name that does not exist",
        "why": "The first of three table scenarios: the target itself is checked.",
        "ts": [sub("router.ts", 'register("network", notifyOncall);',
                   'register("network", notifyOncalll);')],
        "rg": [sub("router.rg", '"network"  -> notify_oncall,',
                   '"network"  -> notify_oncalll,')],
    },
    {
        "id": "handler-wrong-arity",
        "what": "Give a dispatch target an extra parameter",
        "why": "The target is only ever reached through the string-keyed set.",
        "ts": [
            sub("core.ts", "export function escalate(inc: Incident): string {",
                "export function escalate(inc: Incident, retries: number): string {"),
            sub("core.ts", 'return "ESCALATE " + notifyOncall(inc);',
                'return "ESCALATE " + notifyOncall(inc) + retries;'),
        ],
        "rg": [
            sub("core.rg", "fn escalate(inc) {", "fn escalate(inc, retries) {"),
            sub("core.rg", 'return "ESCALATE " + notify_oncall(inc);',
                'return "ESCALATE " + notify_oncall(inc) + str(retries);'),
        ],
    },
    {
        "id": "swap-dispatch-targets",
        "what": "Swap which handler two keys map to",
        "why": "A real behaviour change that breaks no reference. Neither checker "
               "should flag it — included so the benchmark is not only cases "
               "somebody wins.",
        "expected_miss": "No error exists to report: keys are data and bear no required "
                         "relation to handler names, so a swapped pairing is a valid "
                         "program. rigid's hash for the table does move, and the tests "
                         "downstream of it re-execute. See docs/BENCHMARK.md, "
                         "'What can and cannot be checked about a dispatch table'.",
        "ts": [
            sub("router.ts", 'register("network", notifyOncall);\nregister("database", queueNormal);',
                'register("network", queueNormal);\nregister("database", notifyOncall);'),
        ],
        "rg": [
            sub("router.rg", '"network"  -> notify_oncall,\n  "database" -> queue_normal,',
                '"network"  -> queue_normal,\n  "database" -> notify_oncall,'),
        ],
    },

]


def apply(patches, workdir):
    for kind, name, a, b, _ in patches:
        p = workdir / name
        text = p.read_text()
        if kind == "sub":
            if a not in text:
                raise SystemExit("scenario patch did not match in %s:\n%r" % (name, a[:80]))
            text = text.replace(a, b)
        else:
            text = text + a
        p.write_text(text)


def run_ts(workdir):
    # Run from inside the workdir so diagnostics carry short relative paths.
    r = subprocess.run([str(TSC), "-p", "tsconfig.json"], cwd=str(workdir),
                       capture_output=True, text=True)
    out = [l.strip() for l in (r.stdout + r.stderr).splitlines() if l.strip()]
    return r.returncode != 0, out[0] if out else ""


def run_rigid(workdir):
    # Pin the sidecars first: reconciliation can only report that a name's
    # meaning changed if there is a previous resolution to compare against,
    # which is the same thing a committed lockfile gives you in CI.
    subprocess.run([str(RIGID), "check", "--root", str(workdir)],
                   capture_output=True, text=True)
    return None


def check_rigid(workdir):
    r = subprocess.run([str(RIGID), "check", "--root", str(workdir)],
                       capture_output=True, text=True)
    lines = [l.strip() for l in (r.stdout + r.stderr).splitlines()]
    errors = [l for l in lines if l.startswith("error")]
    warnings = [l for l in lines if l.startswith("warning")]
    # Report the error if there is one; a warning only when nothing failed.
    return r.returncode != 0, (errors or warnings or [""])[0]


def main():
    if not RIGID.exists():
        raise SystemExit("build rigid first: cargo build --release")
    if not TSC.exists():
        raise SystemExit("install typescript first: cd bench && npm install typescript@5")

    results = []
    for sc in SCENARIOS:
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            ts, rg = tmp / "ts", tmp / "rg"
            shutil.copytree(ROOT / "base" / "ts", ts)
            shutil.copytree(ROOT / "base" / "rg", rg)
            # Pin the sidecars on the intact program first: this is the state a
            # repo is in before someone makes the change, with lockfiles
            # committed. TypeScript needs no equivalent step.
            run_rigid(rg)
            apply(sc["ts"], ts)
            apply(sc["rg"], rg)
            ts_caught, ts_msg = run_ts(ts)
            rg_caught, rg_msg = check_rigid(rg)
        results.append({**{k: sc[k] for k in ("id", "what", "why")},
                        "expected_miss": sc.get("expected_miss"),
                        "tsc": {"caught": ts_caught, "diagnostic": ts_msg},
                        "rigid": {"caught": rg_caught, "diagnostic": rg_msg}})

    if "--json" in sys.argv:
        print(json.dumps(results, indent=2))
        return

    def mark(r, who):
        if r[who]["caught"]:
            return "caught"
        return "missed*" if r.get("expected_miss") else "MISSED"
    print("\nBreakage detection — tsc --strict vs rigid check\n")
    print("%-20s %-9s %-9s  %s" % ("scenario", "tsc", "rigid", "what changed"))
    print("-" * 92)
    for r in results:
        print("%-21s %-9s %-9s %s" % (r["id"], mark(r, "tsc"), mark(r, "rigid"), r["what"]))
    ts_n = sum(1 for r in results if r["tsc"]["caught"])
    rg_n = sum(1 for r in results if r["rigid"]["caught"])
    print("-" * 92)
    print("%-21s %-9s %-9s of %d scenarios" % ("TOTAL", ts_n, rg_n, len(results)))
    for r in results:
        if r.get("expected_miss"):
            print("\n* %s — expected. %s" % (r["id"], r["expected_miss"]))
    print()

    print("First diagnostic from each checker:\n")
    for r in results:
        print("  %s" % r["id"])
        print("    tsc   : %s" % (r["tsc"]["diagnostic"] or "(none — compiled clean)"))
        print("    rigid : %s" % (r["rigid"]["diagnostic"] or "(none — checked clean)"))
        print()


if __name__ == "__main__":
    main()
