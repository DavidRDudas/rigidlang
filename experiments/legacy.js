// event pipeline — grown organically, as these do
const fs = require("fs");
const R = {};
function reg(k, f) { R[k] = f; }

function log_line(s) { fs.appendFileSync("audit.log", s + "\n"); return s; }
function fmt(rec) { return "[" + rec.kind + "] sev=" + rec.sev; }

function audit(rec) { return log_line("AUDIT " + fmt(rec)); }
function notify(rec) { return "notify:" + fmt(rec); }
function close_out(rec) { return "closed:" + rec.kind; }
function archive(rec) { return "archived:" + fmt(rec); }
// candidate replacement, never wired up
function archive_v2(rec) { return "archived2:" + fmt(rec) + "!"; }

reg("created", audit);
reg("updated", notify);
const h = R; // alias used below
h["closed"] = close_out;
if (true) { reg("archived", archive); }

function escalate(rec) { return audit(rec) && "escalated:" + rec.kind; }
reg("breach", escalate);

function handle(kind, sev) {
  const rec = { kind: kind, sev: sev };
  const f = R[kind];
  if (!f) return "drop:" + kind;
  return f(rec);
}
module.exports = { handle };
if (require.main === module) console.log(handle(process.argv[2], parseInt(process.argv[3] || "1", 10)));
