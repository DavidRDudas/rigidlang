#!/usr/bin/env python3
"""Dispatch-width analyzer, implementing PREREGISTRATION.md P1-P4.
Conservative by construction: anything it cannot bound is either counted
as UNKNOWN (if it is provably an indirect call) or not counted at all."""
import ast, sys, os, json
from collections import defaultdict

class Mod(ast.NodeVisitor):
    def __init__(self, path):
        self.path = path
        self.singleton = 0          # direct calls to a named function
        self.dispatch = []          # widths of bounded dispatch sites
        self.unknown = 0            # unbounded indirect calls
        self.tables = {}            # var -> width  (P1)
        self.registries = defaultdict(int)  # decorator registry -> count (P2)
        self.prefix_methods = defaultdict(int)  # class prefix -> count (P3)

    # ---------- P1: dict literal of str -> callable ----------
    def visit_Assign(self, node):
        if isinstance(node.value, ast.Dict) and node.value.keys:
            keys_are_str = all(isinstance(k, ast.Constant) and isinstance(k.value, str)
                               for k in node.value.keys if k is not None)
            vals_callable = [v for v in node.value.values
                             if isinstance(v, (ast.Name, ast.Attribute, ast.Lambda))]
            if keys_are_str and len(vals_callable) == len(node.value.values) and len(node.value.values) > 0:
                for t in node.targets:
                    name = None
                    if isinstance(t, ast.Name): name = t.id
                    elif isinstance(t, ast.Attribute): name = t.attr
                    if name:
                        self.tables[name] = len(node.value.values)
        self.generic_visit(node)

    # ---------- P2: decorator registries ----------
    def visit_FunctionDef(self, node):
        for d in node.decorator_list:
            f = d.func if isinstance(d, ast.Call) else d
            if isinstance(f, ast.Attribute):
                # e.g. @app.route(...), @registry.register("x")
                if f.attr in ("route","register","command","handler","task","rule","add"):
                    base = f.value.id if isinstance(f.value, ast.Name) else f.attr
                    self.registries[base] += 1
        # P3 material: methods with a common prefix
        if "_" in node.name:
            self.prefix_methods[node.name.split("_")[0] + "_"] += 1
        self.generic_visit(node)

    def visit_Call(self, node):
        f = node.func
        # direct call to a plain name or attribute -> singleton site
        if isinstance(f, ast.Name):
            self.singleton += 1
        elif isinstance(f, ast.Attribute):
            self.singleton += 1
        # P1: table[key](...) -> bounded dispatch
        elif isinstance(f, ast.Subscript):
            base = f.value
            name = base.id if isinstance(base, ast.Name) else (base.attr if isinstance(base, ast.Attribute) else None)
            if name and name in self.tables:
                self.dispatch.append(self.tables[name])
            else:
                self.unknown += 1
        # calling the result of getattr(...)
        elif isinstance(f, ast.Call) and isinstance(f.func, ast.Name) and f.func.id == "getattr":
            self.unknown += 1
        else:
            self.unknown += 1

        # P3/P4: getattr sites, even when not immediately called
        if isinstance(f, ast.Name) and f.id == "getattr" and len(node.args) >= 2:
            a = node.args[1]
            if isinstance(a, ast.BinOp) and isinstance(a.op, ast.Add) and \
               isinstance(a.left, ast.Constant) and isinstance(a.left.value, str):
                self.prefix_hits = getattr(self, "prefix_hits", [])
                self.prefix_hits.append(a.left.value)   # bounded by prefix (P3)
            elif not isinstance(a, ast.Constant):
                self.unknown += 1                        # computed name (P4)
        self.generic_visit(node)

def run(root):
    tot = Mod("")
    agg = {"singleton":0,"dispatch":[],"unknown":0,"registries":defaultdict(int),
           "prefix":defaultdict(int),"prefix_hits":[],"files":0,"parse_fail":0}
    for dirpath, _, files in os.walk(root):
        if any(s in dirpath for s in ("/test","/tests","/docs","/examples")): continue
        for fn in files:
            if not fn.endswith(".py"): continue
            p = os.path.join(dirpath, fn)
            try:
                tree = ast.parse(open(p, encoding="utf-8", errors="replace").read())
            except Exception:
                agg["parse_fail"] += 1; continue
            m = Mod(p); m.visit(tree)
            agg["files"] += 1
            agg["singleton"] += m.singleton
            agg["dispatch"] += m.dispatch
            agg["unknown"] += m.unknown
            for k,v in m.registries.items(): agg["registries"][k] += v
            for k,v in m.prefix_methods.items(): agg["prefix"][k] += v
            agg["prefix_hits"] += getattr(m, "prefix_hits", [])
    return agg

if __name__ == "__main__":
    out = {}
    for name in sys.argv[1:]:
        a = run(os.path.join("repos", name))
        # P3: each getattr(obj, "prefix_"+x) site is bounded by the number of
        # methods sharing that prefix anywhere in the project
        p3 = [a["prefix"].get(px, 0) for px in a["prefix_hits"]]
        p3 = [w for w in p3 if w > 0]
        widths = a["dispatch"] + p3
        # decorator registries: each registry is one dispatch site of its size
        reg = sorted([v for v in a["registries"].values() if v >= 2], reverse=True)
        widths_all = widths + reg
        sites = a["singleton"] + len(widths_all) + a["unknown"]
        out[name] = {
            "files": a["files"], "parse_fail": a["parse_fail"],
            "singleton_sites": a["singleton"],
            "bounded_dispatch_sites": len(widths_all),
            "unknown_sites": a["unknown"],
            "widths": sorted(widths_all, reverse=True),
            "registry_widths": reg[:10],
            "exactness": round(a["singleton"]/sites, 4) if sites else None,
        }
    json.dump(out, open("raw_results.json","w"), indent=1)
    for k,v in out.items():
        w = v["widths"]
        med = sorted(w)[len(w)//2] if w else 0
        print(f"{k:10s} files={v['files']:4d}  singleton={v['singleton_sites']:6d}  "
              f"dispatch={v['bounded_dispatch_sites']:4d}  unknown={v['unknown_sites']:5d}  "
              f"E={v['exactness']}  median_width={med}  max={max(w) if w else 0}")
