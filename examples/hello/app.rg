// app.rg — io through the quarantine door

import { fib } from "./math.rg";

fn main(n) !io {
  print("fib of:");
  print(n);
  return print(fib(n));
}
