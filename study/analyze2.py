#!/usr/bin/env python3
"""v2: measures what a rigid migration would actually face."""
import ast, os, sys, json
from collections import Counter, defaultdict

# repos live next to this script, not next to the shell's cwd
HERE = os.path.dirname(os.path.abspath(__file__))
REPOS = os.path.join(HERE, "repos")

DYN = {"import_string","load_object","import_module","__import__","getattr",
       "import_class","locate","resolve","entry_point","load"}

def scan(root):
    r = {"files":0,"direct":0,"method":0,"table_calls":0,"dynimport":0,
         "registry_sizes":Counter(),"dict_table_sizes":Counter(),
         "getattr_computed":0,"getattr_prefix":0}
    reg = defaultdict(int)
    for dp,_,fs in os.walk(root):
        if any(s in dp for s in ("/test","/tests","/docs","/examples","/.git")): continue
        for fn in fs:
            if not fn.endswith(".py"): continue
            try: t = ast.parse(open(os.path.join(dp,fn),encoding="utf-8",errors="replace").read())
            except Exception: continue
            r["files"] += 1
            tables = {}
            for n in ast.walk(t):
                # str-keyed dict of callables = a rigid `table`
                if isinstance(n, ast.Dict) and n.keys and all(
                        isinstance(k, ast.Constant) and isinstance(k.value,str)
                        for k in n.keys if k is not None):
                    vals = n.values
                    if vals and all(isinstance(v,(ast.Name,ast.Attribute,ast.Lambda,ast.Tuple)) for v in vals):
                        r["dict_table_sizes"][len(vals)] += 1
                if isinstance(n, ast.Assign) and isinstance(n.value, ast.Dict):
                    for tg in n.targets:
                        nm = tg.id if isinstance(tg,ast.Name) else (tg.attr if isinstance(tg,ast.Attribute) else None)
                        if nm: tables[nm] = len(n.value.values)
                # decorator registries
                if isinstance(n,(ast.FunctionDef,ast.AsyncFunctionDef,ast.ClassDef)):
                    for d in n.decorator_list:
                        f = d.func if isinstance(d,ast.Call) else d
                        if isinstance(f,ast.Attribute) and f.attr in ("route","register","command","handler","task","rule","add","subscribe","on"):
                            base = f.value.id if isinstance(f.value,ast.Name) else f.attr
                            reg[(root,base)] += 1
                if isinstance(n, ast.Call):
                    f = n.func
                    if isinstance(f, ast.Name):
                        if f.id in DYN: r["dynimport"] += 1
                        else: r["direct"] += 1
                    elif isinstance(f, ast.Attribute):
                        if f.attr in DYN: r["dynimport"] += 1
                        else: r["method"] += 1
                    elif isinstance(f, ast.Subscript):
                        b = f.value
                        nm = b.id if isinstance(b,ast.Name) else (b.attr if isinstance(b,ast.Attribute) else None)
                        if nm and nm in tables: r["table_calls"] += 1
                        else: r["dynimport"] += 1
                    else:
                        r["dynimport"] += 1
                    if isinstance(f, ast.Name) and f.id == "getattr" and len(n.args)>=2:
                        a = n.args[1]
                        if isinstance(a,ast.BinOp) and isinstance(a.left,ast.Constant): r["getattr_prefix"] += 1
                        elif not isinstance(a,ast.Constant): r["getattr_computed"] += 1
    r["registry_sizes"] = Counter(v for k,v in reg.items() if v>=2)
    return r

names = sys.argv[1:] or ["flask","scrapy","celery","pygments","httpie"]
out={}
for name in names:
    r = scan(os.path.join(REPOS, name))
    sizes = list(r["dict_table_sizes"].elements()) + list(r["registry_sizes"].elements())
    sizes.sort()
    med = sizes[len(sizes)//2] if sizes else 0
    p90 = sizes[int(len(sizes)*0.9)] if sizes else 0
    out[name]={"files":r["files"],"direct":r["direct"],"method":r["method"],
               "table_calls":r["table_calls"],"dynamic_unbounded":r["dynimport"],
               "n_tables":len(sizes),"median_table":med,"p90_table":p90,
               "max_table":max(sizes) if sizes else 0,
               "sizes_hist":dict(Counter(sizes)),
               "getattr_computed":r["getattr_computed"],"getattr_prefix":r["getattr_prefix"]}
    print(f"{name:9s} tables={len(sizes):4d} median={med:3d} p90={p90:4d} max={max(sizes) if sizes else 0:4d} "
          f"| direct={r['direct']:6d} method={r['method']:7d} unbounded={r['dynimport']:5d}")
json.dump(out,open(os.path.join(HERE,"raw_results_v2.json"),"w"),indent=1)
