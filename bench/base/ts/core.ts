export type Incident = { kind: string; impact: number; urgency: number };
export type Handler = (inc: Incident) => string;

export function severity(impact: number, urgency: number): number {
  return impact * urgency;
}

export function formatIncident(inc: Incident): string {
  return "[" + inc.kind + "] sev=" + severity(inc.impact, inc.urgency);
}

export function notifyOncall(inc: Incident): string {
  return "PAGE " + formatIncident(inc);
}

export function queueNormal(inc: Incident): string {
  return "QUEUE " + formatIncident(inc);
}

export function escalate(inc: Incident): string {
  return "ESCALATE " + notifyOncall(inc);
}
