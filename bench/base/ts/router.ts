import { Handler, notifyOncall, queueNormal, escalate, severity } from "./core.js";

// Plugin-style registry populated at load time.
const registry: Record<string, Handler> = {};
function register(key: string, fn: Handler): void {
  registry[key] = fn;
}

register("network", notifyOncall);
register("database", queueNormal);
register("breach", escalate);

export function route(kind: string, impact: number, urgency: number): string {
  const fn = registry[kind];
  if (!fn) return "drop:" + kind;
  return fn({ kind, impact, urgency });
}

export function worstCase(): number {
  return severity(5, 5);
}
