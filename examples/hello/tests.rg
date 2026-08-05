// tests.rg — test_* definitions; results cached by content hash

import { fib, norm } from "./math.rg";
import { is_even } from "./logic.rg";

fn test_fib() {
  return fib(10) == 55;
}

fn test_norm() {
  return norm(250) == 200;
}

fn test_even() {
  return is_even(40);
}
