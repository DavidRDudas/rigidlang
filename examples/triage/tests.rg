// tests.rg — the pure core is testable and cache-provable

import { severity, is_p1, clampi } from "./score.rg";

fn test_severity_clamps() {
  return severity(5, 5) == 9;
}

fn test_severity_floor() {
  return severity(0, 3) == 1;
}

fn test_p1_boundary() {
  return if (is_p1(8)) is_p1(9) else 0;
}

fn test_clamp_passthrough() {
  return clampi(4, 1, 9) == 4;
}
