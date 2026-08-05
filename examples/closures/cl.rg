// Closures, with the graph intact.
//
// Every `fn(x) { ... }` is lifted to a real top-level definition — it has a
// content hash, `rdeps` reaches it, and the quarantine still bounds where it
// can be invoked. What a closure adds over `&name` is the captured
// environment, which becomes the lifted definition's leading parameters.
//
// The synthetic names (`adder$0`) are the compiler's; `$` cannot start an
// identifier, so they never collide with one an author wrote.

fn apply_all(f, xs) {
  let out = [];
  for (x in xs) {
    out = push(out, dyn { adder$0, scaler$0, offset$0 }(f, x));
  }
  return out;
}

fn adder(n) {
  return fn(x) { return x + n; };
}

fn scaler(k) {
  return fn(x) { return x * k; };
}

fn offset(a, b) {
  return fn(x) { return x + a - b; };
}

fn sum(xs) {
  let acc = 0;
  for (x in xs) {
    acc = acc + x;
  }
  return acc;
}

fn add_then_sum(n, k) {
  return sum(apply_all(adder(n), [1, 2, 3, k]));
}

fn scale_then_sum(k) {
  return sum(apply_all(scaler(k), [1, 2, 3]));
}

fn two_captures(a, b) {
  return sum(apply_all(offset(a, b), [10, 20]));
}

fn test_adder() { return add_then_sum(10, 4) == 50; }
fn test_scaler() { return scale_then_sum(3) == 18; }
fn test_two_captures() { return two_captures(5, 2) == 36; }
