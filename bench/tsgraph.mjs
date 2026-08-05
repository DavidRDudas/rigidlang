// Emit a dependency graph for the TypeScript/JavaScript baseline using the
// TypeScript compiler API. This is condition D's input: an existing, standard
// tool doing its best on the same program, so the pilot can tell "a spec helps"
// apart from "rigid's spec specifically helps".
import ts from "./node_modules/typescript/lib/typescript.js";
import { readFileSync } from "fs";

const file = process.argv[2];
const src = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.ES2020, true,
  file.endsWith(".ts") ? ts.ScriptKind.TS : ts.ScriptKind.JS);

const fns = new Map();       // name -> { calls:Set, io:boolean }
const topLevel = [];

function fnName(node) {
  if (ts.isFunctionDeclaration(node) && node.name) return node.name.text;
  return null;
}

// Pass 1: collect function declarations and the identifiers they call.
function visit(node, current) {
  const name = fnName(node);
  if (name) {
    fns.set(name, { calls: new Set(), io: false });
    topLevel.push(name);
    node.body && node.body.forEachChild(c => visit(c, name));
    return;
  }
  if (current && ts.isCallExpression(node)) {
    const e = node.expression;
    if (ts.isIdentifier(e)) fns.get(current).calls.add(e.text);
    // Treat fs/console writes as the io boundary, the same way rigid's
    // extern declarations mark it.
    if (ts.isPropertyAccessExpression(e)) {
      const root = e.expression.getText();
      if (root === "fs" || root === "console") fns.get(current).io = true;
    }
  }
  node.forEachChild(c => visit(c, current));
}
src.forEachChild(c => visit(c, null));

// Pass 2: propagate io along the call graph to a fixed point.
let changed = true;
while (changed) {
  changed = false;
  for (const [n, info] of fns) {
    if (info.io) continue;
    for (const c of info.calls) {
      if (fns.get(c)?.io) { info.io = true; changed = true; break; }
    }
  }
}

const out = { file, functions: {} };
for (const n of topLevel) {
  const info = fns.get(n);
  const callers = topLevel.filter(o => fns.get(o).calls.has(n));
  out.functions[n] = { calls: [...info.calls].filter(c => fns.has(c)).sort(), calledBy: callers.sort(), performsIO: info.io };
}
console.log(JSON.stringify(out, null, 2));
