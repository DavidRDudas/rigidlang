import { notify_oncall, queue_normal, escalate, severity } from "./core.rg";

table registry {
  "network"  -> notify_oncall,
  "database" -> queue_normal,
  "breach"   -> escalate,
}

fn route(kind, impact, urgency) {
  return registry[kind]({ kind: kind, impact: impact, urgency: urgency });
}

fn worst_case() {
  return severity(5, 5);
}
