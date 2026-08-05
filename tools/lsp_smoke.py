#!/usr/bin/env python3
"""Scripted LSP client: the editor this environment doesn't have.
Drives `rigid lsp` over stdio and asserts on real protocol traffic."""
import json, subprocess, sys, os

root = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "examples/migration")
srv = subprocess.Popen([sys.argv[2] if len(sys.argv) > 2 else "./target/release/rigid",
                        "lsp", "--root", root],
                       stdin=subprocess.PIPE, stdout=subprocess.PIPE)

def send(method, params=None, id=None):
    msg = {"jsonrpc": "2.0", "method": method}
    if params is not None: msg["params"] = params
    if id is not None: msg["id"] = id
    b = json.dumps(msg).encode()
    srv.stdin.write(f"Content-Length: {len(b)}\r\n\r\n".encode() + b)
    srv.stdin.flush()

def recv():
    header = b""
    while not header.endswith(b"\r\n\r\n"):
        c = srv.stdout.read(1)
        if not c: return None
        header += c
    n = int([l for l in header.decode().split("\r\n") if l.startswith("Content-Length:")][0].split(":")[1])
    return json.loads(srv.stdout.read(n))

def wait_for(pred, limit=50):
    for _ in range(limit):
        m = recv()
        if m is None: raise SystemExit("server closed early")
        if pred(m): return m
    raise SystemExit("expected message not seen")

def uri(rel): return f"file://{os.path.join(root, rel)}"

ok = 0
def check(name, cond):
    global ok
    print(("PASS  " if cond else "FAIL  ") + name)
    if not cond: sys.exit(1)
    ok += 1

send("initialize", {"rootUri": f"file://{root}"}, id=1)
init = wait_for(lambda m: m.get("id") == 1)
caps = init["result"]["capabilities"]
check("initialize advertises hover/definition/references/codeAction",
      all(caps.get(k) for k in ("hoverProvider","definitionProvider","referencesProvider","codeActionProvider")))

send("initialized", {})
first = wait_for(lambda m: m.get("method") == "textDocument/publishDiagnostics")
check("diagnostics published on initialized", "uri" in first["params"])

# hover on `audit` in the table entry line of handlers.rg
handlers = open(os.path.join(root, "handlers.rg")).read().splitlines()
tline = next(i for i,l in enumerate(handlers) if '"task.created"' in l)
tcol = handlers[tline].index("audit")
send("textDocument/hover", {"textDocument": {"uri": uri("handlers.rg")},
                            "position": {"line": tline, "character": tcol}}, id=2)
hov = wait_for(lambda m: m.get("id") == 2)
check("hover shows content identity (sha256)", "sha256:" in json.dumps(hov["result"]))

# definition from the same spot lands on `fn audit`
send("textDocument/definition", {"textDocument": {"uri": uri("handlers.rg")},
                                 "position": {"line": tline, "character": tcol}}, id=3)
d = wait_for(lambda m: m.get("id") == 3)["result"]
defline = next(i for i,l in enumerate(handlers) if l.startswith("fn audit"))
check("definition resolves through the table entry", d["range"]["start"]["line"] == defline)

# references of audit: def site + table entry = exact, graph-derived
send("textDocument/references", {"textDocument": {"uri": uri("handlers.rg")},
                                 "position": {"line": defline, "character": 4}}, id=4)
refs = wait_for(lambda m: m.get("id") == 4)["result"]
lines = sorted(r["range"]["start"]["line"] for r in refs)
check("references are exact through the table (def site + entry)",
      len(refs) == 2 and defline in lines and tline in lines)

# didChange: rename the TABLE in the unsaved buffer -> the importing file
# (router.rg) gets a live E0201 carrying the identity hint, cross-file,
# without anything touching disk
broken = "\n".join(handlers).replace("table lifecycle", "table lifecycle_v2") + "\n"
send("textDocument/didChange", {"textDocument": {"uri": uri("handlers.rg")},
                                "contentChanges": [{"text": broken}]})
diag = wait_for(lambda m: m.get("method") == "textDocument/publishDiagnostics"
                and m["params"]["uri"].endswith("router.rg")
                and any(d.get("code") == "E0201" and "lifecycle_v2" in d.get("message","")
                        for d in m["params"]["diagnostics"]))
check("unsaved rename in one buffer -> live identity-hint E0201 in the importer", True)

send("shutdown", None, id=9); wait_for(lambda m: m.get("id") == 9)
send("exit")
print(f"{ok} LSP checks passed")
