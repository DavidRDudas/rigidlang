// `if` as a statement and `for` over a sequence. Both are real AST nodes with
// their own canonical tags, not sugar — so `fmt` gives them back as written.

fn classify(n) {
  if (n < 0) {
    return "negative";
  } else if (n == 0) {
    return "zero";
  } else {
    return "positive";
  }
}

fn total(xs) {
  let acc = 0;
  for (x in xs) {
    acc = acc + x;
  }
  return acc;
}

fn spell(s) {
  let out = "";
  for (c in s) {
    out = out + c + ".";
  }
  return out;
}

fn evens_upto(n) {
  let out = [];
  let i = 0;
  while (i < n) {
    if (i % 2 == 0) {
      out = push(out, i);
    }
    i = i + 1;
  }
  return out;
}

fn sum_upto(n) {
  let xs = [];
  let i = 1;
  while (i <= n) {
    xs = push(xs, i);
    i = i + 1;
  }
  return total(xs);
}

fn test_sum_upto() { return sum_upto(4) == 10; }
fn test_classify() { return classify(-1) == "negative"; }
fn test_total() { return total([1, 2, 3, 4]) == 10; }
fn test_spell() { return spell("ab") == "a.b."; }
fn test_evens() { return len(evens_upto(10)) == 5; }
