fn severity(impact, urgency) {
  return impact * urgency;
}

fn format_incident(inc) {
  return "[" + inc.kind + "] sev=" + str(severity(inc.impact, inc.urgency));
}

fn notify_oncall(inc) {
  return "PAGE " + format_incident(inc);
}

fn queue_normal(inc) {
  return "QUEUE " + format_incident(inc);
}

fn escalate(inc) {
  return "ESCALATE " + notify_oncall(inc);
}
