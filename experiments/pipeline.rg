extern fn log_line(s) !io = "console.error";

fn fmt(rec) { return "[" + rec.kind + "] sev=" + str(rec.sev); }

fn audit(rec) !io { log_line("AUDIT " + fmt(rec)); return "AUDIT " + fmt(rec); }
fn notify(rec) { return "notify:" + fmt(rec); }
fn close_out(rec) { return "closed:" + rec.kind; }
fn archive(rec) { return "archived:" + fmt(rec); }
fn archive_v2(rec) { return "archived2:" + fmt(rec) + "!"; }

fn escalate(rec) !io { audit(rec); return "escalated:" + rec.kind; }

table handlers {
  "created"  -> audit,
  "updated"  -> notify,
  "closed"   -> close_out,
  "archived" -> archive,
  "breach"   -> escalate,
}

fn handle(kind, sev) !io {
  let rec = { kind: kind, sev: sev };
  return handlers[kind](rec);
}
