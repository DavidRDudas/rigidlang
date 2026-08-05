// math.rg — depends on util.rg

import { clamp, scale } from "./util.rg";

fn norm(x) {
  let bounded = clamp(x, 0, 100);
  return scale(bounded, 2);
}

fn fib(n) {
  return if (n < 2) n else fib(n - 1) + fib(n - 2);
}
