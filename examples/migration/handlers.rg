// handlers.rg — the registry, migrated: strings stay strings,
// the value set is closed, effects are declared.

import { priority, breach_risk } from "./core.rg";

fn audit(rec) !io {
  return print("audit: " + rec.class);
}

fn notify(rec) !io {
  return print("notify assignee, priority " + str(priority(rec.impact, rec.urgency)));
}

fn archive(rec) !io {
  return print("archived " + rec.class);
}

fn page_oncall(rec) !io {
  return print("PAGE risk=" + str(breach_risk(rec.age, rec.sla)));
}

fn queue_oncall(rec) !io {
  return print("queued for oncall review");
}

fn escalate(rec) !io {
  let h = if (breach_risk(rec.age, rec.sla) > 7) &page_oncall else &queue_oncall;
  return dyn { page_oncall, queue_oncall }(h, rec);
}

table lifecycle {
  "task.created" -> audit,
  "task.updated" -> notify,
  "task.closed"  -> archive,
  "sla.breach"   -> escalate,
}
