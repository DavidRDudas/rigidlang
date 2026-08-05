// externs: foreign functions as declared boundary nodes
extern fn jmax(a, b) = "Math.max";
extern fn stamp() !io = "Date.now";

fn pick(a, b) {
  return jmax(a, b);
}

fn report(a, b) !io {
  print("winner: " + str(pick(a, b)));
  return stamp();
}

fn test_pick_pure() { return 1; }
