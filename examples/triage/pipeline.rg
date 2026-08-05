// pipeline.rg — the ServiceNow parable, written in rigid:
// string-keyed routing, quarantined escalation, effects declared,
// and the whole thing sits in an exact reference graph.

import { severity, is_p1 } from "./score.rg";

fn log(tag, v) !io {
  print(tag);
  return print(v);
}

fn notify_oncall(sev) !io {
  return log("PAGE oncall, severity:", sev);
}

fn queue_normal(sev) !io {
  return log("queued, severity:", sev);
}

fn escalate(sev) !io {
  let h = if (is_p1(sev)) &notify_oncall else &queue_normal;
  return dyn { notify_oncall, queue_normal }(h, sev);
}

table routes {
  "network"  -> route_network,
  "database" -> route_database,
  "access"   -> route_access,
}

fn route_network(i, u) !io {
  return escalate(severity(i, u) + 1);
}

fn route_database(i, u) !io {
  return escalate(severity(i, u));
}

fn route_access(i, u) !io {
  return escalate(severity(i, u) - 2);
}

fn triage(category, impact, urgency) !io {
  print("incident: " + category);
  return routes[category](impact, urgency);
}
