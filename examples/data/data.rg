fn sum(xs, i) {
  return if (i >= len(xs)) 0 else xs[i] + sum(xs, i + 1);
}

fn maxi(xs, i, best) {
  return if (i >= len(xs)) best
    else maxi(xs, i + 1, if (xs[i] > best) xs[i] else best);
}

fn build(n, acc) {
  return if (n == 0) acc else build(n - 1, push(acc, n * n));
}

fn make_incident(cat, sev) {
  return { category: cat, severity: sev, tags: ["auto", "v5"] };
}

fn test_sum() { return sum([1, 2, 3, 4], 0) == 10; }
fn test_max() { return maxi([3, 9, 2], 0, 0) == 9; }
fn test_build() { return len(build(4, [])) == 4; }
fn test_record() {
  let inc = make_incident("network", 7);
  return if (inc.severity == 7) inc.tags[1] == "v5" else 0;
}
fn test_concat() { return len([1] + [2, 3]) == 3; }
fn test_strindex() { return "abc"[1] == "b"; }

fn main() !io {
  let inc = make_incident("network", 7);
  print(inc);
  print(inc.category + " squares:");
  return print(build(5, []));
}
