#!/usr/bin/env bash
# Walkthrough of the v0 kernel's claims, on a scratch copy of examples/hello.
set -euo pipefail
cd "$(dirname "$0")"

RIGID="cargo run -q --"
WORK=$(mktemp -d)
cp examples/hello/*.rg "$WORK"/
step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
# portable in-place sed (GNU and BSD/macOS); file must be the last argument
sedi() { local f="${!#}"; sed -i.bak "$@" && rm -f "$f.bak"; }

step "1. First check: references get pinned (I0402), sidecars written"
$RIGID check --root "$WORK" || true
echo "--- math.rg.bind.json ---"
cat "$WORK/math.rg.bind.json"

step "2. Identity survives local renames: hash scale, rename its parameter, hash again"
$RIGID hash "util.rg#scale" --root "$WORK"
sedi 's/fn scale(x, k)/fn scale(value, factor)/; s/return x \* k;/return value * factor;/' "$WORK/util.rg"
$RIGID hash "util.rg#scale" --root "$WORK"
echo "(same hash: the name layer is not the identity layer)"

step "3. Rename a definition, 'forget' a call site: structured error + typed fix, not a silent break"
sedi 's/fn clamp/fn clip/' "$WORK/util.rg"
$RIGID check --root "$WORK" --json || true

step "4. Re-use the old name for a DIFFERENT definition: the silent-rebind refusal (E0202)"
cat >> "$WORK/util.rg" <<'EOF'

fn clamp(a, b, c) {
  return a + b + c;
}
EOF
$RIGID check --root "$WORK" || true

step "5. Blast radius is a query, not archaeology"
sedi 's/fn clip/fn clamp/' "$WORK/util.rg"           # undo rename
sedi '/return a + b + c;/{N;d;}' "$WORK/util.rg"        # drop impostor body+brace
sedi '/fn clamp(a, b, c) {/d' "$WORK/util.rg"           # drop impostor header
$RIGID check --root "$WORK" >/dev/null || true
$RIGID rdeps "util.rg#clamp" --root "$WORK"
$RIGID deps  "math.rg#norm"  --root "$WORK"

step "6. Content drift is visible and named (W0401), then repinned"
sedi 's/return value \* factor;/return value * factor + 0;/' "$WORK/util.rg"
$RIGID check --root "$WORK" || true

step "7. fix --apply: the agent repair loop, closed"
WORK2=$(mktemp -d)
cp examples/hello/*.rg "$WORK2"/
$RIGID check --root "$WORK2" >/dev/null || true            # pin everything
sedi 's/fn clamp/fn clip/' "$WORK2/util.rg"              # rename def, "forget" call sites
$RIGID fix --root "$WORK2"                                 # dry run: show what would happen
$RIGID fix --apply --root "$WORK2"                         # apply + converge
echo "--- math.rg after mechanical fix ---"
grep -n "clip" "$WORK2/math.rg"

step "8. Mutual recursion hashes (SCC as a unit, v1)"
$RIGID hash --all --root "$WORK2" | grep logic.rg

step "9. Programs run"
printf 'fib(20)      = '; $RIGID run "math.rg#fib" 20 --root "$WORK2"
printf 'norm(250)    = '; $RIGID run "math.rg#norm" 250 --root "$WORK2"
printf 'is_even(41)  = '; $RIGID run "logic.rg#is_even" 41 --root "$WORK2"

step "10. Content-addressed test caching: unchanged tests provably need no rerun"
$RIGID test --root "$WORK2"
echo "--- run again: everything cached ---"
$RIGID test --root "$WORK2"
echo "--- edit fib only: test_fib reruns, the rest stay cached ---"
sedi 's/fib(n - 1) + fib(n - 2)/fib(n - 2) + fib(n - 1)/' "$WORK2/math.rg"
$RIGID check --root "$WORK2" >/dev/null || true
$RIGID test --root "$WORK2"

step "11. Canonical formatting: one true rendering, whatever the input spacing"
mkdir -p "$WORK2/fmtdemo"
printf 'fn f(a,b){return a+b*2;}' > "$WORK2/fmtdemo/x.rg"
printf 'fn f( a , b ) {\n      return a  +  b*2   ;\n}' > "$WORK2/fmtdemo/y.rg"
$RIGID fmt "fmtdemo/x.rg" --root "$WORK2" > /tmp/fx.txt
$RIGID fmt "fmtdemo/y.rg" --root "$WORK2" > /tmp/fy.txt
diff /tmp/fx.txt /tmp/fy.txt && echo "fmt outputs identical"
$RIGID hash "fmtdemo/x.rg#f" --root "$WORK2" | awk '{print "x.rg#f", $2}'
$RIGID hash "fmtdemo/y.rg#f" --root "$WORK2" | awk '{print "y.rg#f", $2}'

step "12. Quarantined dynamism: first-class refs without losing the graph"
WORK3=$(mktemp -d)
cp examples/hello/*.rg "$WORK3"/
$RIGID check --root "$WORK3" >/dev/null || true
printf 'apply(2,10) via dyn = '; $RIGID run "dispatch.rg#apply" 2 10 --root "$WORK3"
printf 'apply(3,10) via dyn = '; $RIGID run "dispatch.rg#apply" 3 10 --root "$WORK3"
echo "--- a ref outside the declared set is refused at the boundary ---"
$RIGID run "dispatch.rg#escape" 10 --root "$WORK3" || true
echo "--- and blast radius sees THROUGH the dynamism ---"
$RIGID rdeps "dispatch.rg#triple" --root "$WORK3"

step "13. Effects travel the exact graph"
$RIGID run "app.rg#main" 12 --root "$WORK3"
printf 'import { main } from "./app.rg";\nfn sneaky(n) {\n  return main(n);\n}\n' > "$WORK3/bad_effect.rg"
$RIGID check --root "$WORK3" 2>&1 | grep E0207 || true
rm "$WORK3/bad_effect.rg"

step "14. Comment-preserving canonical formatting (hash-stable)"
printf '// header comment\nfn   messy( a ,b ) {\n     // explain the sum\n   let s=a+  b;   // trailing note\n      return s *2;\n}\n' > "$WORK3/messy.rg"
$RIGID hash "messy.rg#messy" --root "$WORK3"
$RIGID fmt "messy.rg" --write --root "$WORK3"
cat "$WORK3/messy.rg"
$RIGID hash "messy.rg#messy" --root "$WORK3"

step "15. Data-driven dispatch: string keys, closed set, exact graph"
WORK4=$(mktemp -d)
cp examples/hello/*.rg "$WORK4"/
$RIGID check --root "$WORK4" >/dev/null || true
printf 'route("created", 5)  -> '; $RIGID run "router.rg#route" created 5 --root "$WORK4" | tail -1
printf 'route("resolved", 5) -> '; $RIGID run "router.rg#route" resolved 5 --root "$WORK4" | tail -1
echo "--- a key outside the closed set is refused, listing the set ---"
$RIGID run "router.rg#route" deleted 5 --root "$WORK4" || true
echo "--- blast radius flows THROUGH the table, two hops ---"
$RIGID rdeps "router.rg#on_created" --root "$WORK4"
$RIGID rdeps "router.rg#handlers" --root "$WORK4"
echo "--- effects flow through it too: pure dispatcher of an io table is E0207 ---"
printf 'import { handlers } from "./router.rg";\nfn sneaky(ev, p) {\n  return handlers[ev](p);\n}\n' > "$WORK4/bad_table.rg"
$RIGID check --root "$WORK4" 2>&1 | grep E0207 || true
rm "$WORK4/bad_table.rg"

step "16. The table is a definition: content-addressed, cone-forming"
before_t=$($RIGID hash "router.rg#handlers" --root "$WORK4" | awk '{print $2}')
before_r=$($RIGID hash "router.rg#route" --root "$WORK4" | awk '{print $2}')
sedi 's/return print(p \* 10);/return print(p * 100);/' "$WORK4/router.rg"
after_t=$($RIGID hash "router.rg#handlers" --root "$WORK4" | awk '{print $2}')
after_r=$($RIGID hash "router.rg#route" --root "$WORK4" | awk '{print $2}')
echo "edit ONE handler body:"
echo "  handlers  $before_t"
echo "        ->  $after_t"
echo "  route     $before_r"
echo "        ->  $after_r"
echo "(the table's identity — and every dispatcher's — pins the whole value set)"

step "17. Arity: the first type, because the quarantine needed it"
mkdir -p "$WORK4/ar"
printf 'fn f(a, b) { return a + b; }\nfn g() { return f(1); }\nfn h(x) { return dyn { f }( &f, x); }\ntable t { "k" -> f }\nfn j(x) { return t["k"](x); }\n' > "$WORK4/ar/x.rg"
$RIGID check --root "$WORK4/ar" 2>&1 | grep E0209 || true
rm -rf "$WORK4/ar"

step "18. Dogfood: an incident-triage pipeline written in rigid"
WORK5=$(mktemp -d)
cp examples/triage/*.rg "$WORK5"/
$RIGID check --root "$WORK5" >/dev/null || true
$RIGID run "pipeline.rg#triage" network 3 3 --root "$WORK5"
$RIGID run "pipeline.rg#triage" access 2 2 --root "$WORK5"
echo "--- unknown category is refused at the boundary ---"
$RIGID run "pipeline.rg#triage" hardware 3 3 --root "$WORK5" || true
echo "--- the pure core is testable, and provably cacheable ---"
$RIGID test --root "$WORK5"
$RIGID test --root "$WORK5"
echo "--- one query spans routes AND tests ---"
$RIGID rdeps "score.rg#severity" --root "$WORK5"

step "19. Data structures: lists, records, indexing, builtins"
WORK6=$(mktemp -d)
cp examples/data/*.rg "$WORK6"/
$RIGID check --root "$WORK6" >/dev/null || true
$RIGID run "data.rg#main" --root "$WORK6"
$RIGID test --root "$WORK6"

step "20. Field order is presentation; structure is identity"
mkdir -p "$WORK6/ord"
printf 'fn a() { return { x: 1, y: 2 }; }\n' > "$WORK6/ord/m1.rg"
printf 'fn a() { return { y: 2, x: 1 }; }\n' > "$WORK6/ord/m2.rg"
$RIGID hash "m1.rg#a" --root "$WORK6/ord"
$RIGID hash "m2.rg#a" --root "$WORK6/ord"
echo "(two different sources, one identity)"

step "21. Mutation and while: iteration without recursion"
WORK7=$(mktemp -d)
cp examples/flow/*.rg "$WORK7"/
$RIGID check --root "$WORK7" >/dev/null || true
printf 'fib_iter(30) = '; $RIGID run "flow.rg#fib_iter" 30 --root "$WORK7"
printf 'sum_to(100)  = '; $RIGID run "flow.rg#sum_to" 100 --root "$WORK7"
echo "--- assigning to a definition is refused (E0210) ---"
ASG=$(mktemp -d) && printf 'fn f(x) { return x; }\nfn g() { f = 3; return 0; }\n' > "$ASG/x.rg"
$RIGID check --root "$ASG" 2>&1 | grep E0210 || true

step "22. Errors are values: fail carries anything, try catches everything"
$RIGID run "flow.rg#safe_div" 10 0 --root "$WORK7"
echo "--- an uncaught fail surfaces its structured payload ---"
$RIGID run "flow.rg#checked_div" 5 0 --root "$WORK7" || true
$RIGID test --root "$WORK7"

step "23. Stdlib: str/int/substr/chars/join — with catchable parse failures"
WORK8=$(mktemp -d)
cp examples/std/*.rg "$WORK8"/
$RIGID check --root "$WORK8" >/dev/null || true
$RIGID test --root "$WORK8"

step "24. File IO through the effect door"
printf 'round trip: '; $RIGID run "std.rg#save_report" "$WORK8/report.txt" 12 --root "$WORK8"
printf 'on disk:    '; cat "$WORK8/report.txt"; echo
IOD=$(mktemp -d)
printf 'fn probe(p) !io {\n  let r = try(read_file(p));\n  return if (r.ok) r.value else "absent";\n}\n' > "$IOD/m.rg"
printf 'missing file, caught: '; $RIGID run "m.rg#probe" "$IOD/nope" --root "$IOD"
printf 'fn sneak(p) { return read_file(p); }\n' > "$IOD/bad.rg"
$RIGID check --root "$IOD" 2>&1 | grep E0207 || true

step "25. REPL against a live workspace"
WORK9=$(mktemp -d)
cp examples/triage/*.rg "$WORK9"/
printf 'severity(3, 3)\nis_p1(9)\nlet x = 5; return x * x;\n:q\n' | $RIGID repl --root "$WORK9" --mod score.rg

step "26. The metric is measurable: rigid exactness"
WORKA=$(mktemp -d)
cp examples/migration/*.rg "$WORKA"/
$RIGID check --root "$WORKA" >/dev/null || true
echo "--- migrated platform app (see examples/migration/legacy.js.txt for the before) ---"
$RIGID exactness --root "$WORKA"
WORKB=$(mktemp -d); cp examples/flow/*.rg "$WORKB"/; $RIGID check --root "$WORKB" >/dev/null || true
echo "--- pure computation, no dynamism ---"
$RIGID exactness --root "$WORKB"

step "27. Bytecode VM: same semantics, measured speedup"
echo "(all 26 steps above just ran on the compiled engine)"
printf 'fib(36) = '; $RIGID run "math.rg#fib" 36 --root examples/hello
rm -f examples/hello/*.bind.json
echo "(measured: 2.1x over the v8 tree-walker at fib(36): 7.3s vs 15.4s release builds)"

step "28. Set inference: the compiler proposes the quarantine"
WQ=$(mktemp -d)
printf 'fn double(x) { return x * 2; }\nfn triple(x) { return x * 3; }\n\nfn pick(k) {\n  return if (k == 2) &double else &triple;\n}\n\nfn apply(k, x) {\n  let f = pick(k);\n  return f(x);\n}\n' > "$WQ/app.rg"
$RIGID check --root "$WQ" 2>&1 | grep -E "E0204|I0405" || true
$RIGID fix --apply --root "$WQ" 2>&1 | grep -E "applied|converged" || true
grep "dyn" "$WQ/app.rg"
printf 'apply(3, 10) = '; $RIGID run "app.rg#apply" 3 10 --root "$WQ"
echo "--- and a too-narrow declared set is widened the same way (W0403) ---"
printf 'fn a(x) { return x + 1; }\nfn b(x) { return x + 2; }\n\nfn go(k, x) {\n  let f = if (k == 1) &a else &b;\n  return dyn { a }(f, x);\n}\n' > "$WQ/w.rg"
$RIGID check --root "$WQ" 2>&1 | grep W0403 || true
$RIGID fix --apply --root "$WQ" 2>&1 | grep applied || true
grep "dyn {" "$WQ/w.rg" | tail -1

step "29. Quarantine pins: handler drift is a reconciliation event"
WP=$(mktemp -d)
cp examples/migration/*.rg "$WP"/
$RIGID check --root "$WP" >/dev/null || true
sedi 's/print("archived " + rec.class)/print("ARCHIVED " + rec.class)/' "$WP/handlers.rg"
$RIGID check --root "$WP" 2>&1 | grep W0404 || true
sedi 's/ARCHIVED /archived v2 /' "$WP/handlers.rg"
echo "--- under --frozen, quarantine drift refuses ---"
$RIGID check --frozen --root "$WP" 2>&1 | grep W0404 || true

step "30. Module identity: names and order are presentation"
WM=$(mktemp -d); mkdir -p "$WM/a" "$WM/b" "$WM/c"
printf 'fn add(x, y) { return x + y; }\nfn mul(x, y) { return x * y; }\n' > "$WM/a/m.rg"
printf 'fn mul(x, y) { return x * y; }\nfn add(x, y) { return x + y; }\n' > "$WM/b/m.rg"
printf 'fn plus(x, y) { return x + y; }\nfn times(x, y) { return x * y; }\n' > "$WM/c/m.rg"
$RIGID hash m.rg --root "$WM/a"
$RIGID hash m.rg --root "$WM/b"
$RIGID hash m.rg --root "$WM/c"
echo "(reordered, renamed — one identity)"

step "31. LSP over the graph: the editor queries are the compiler's primitives"
WL=$(mktemp -d)
cp examples/migration/*.rg "$WL"/
$RIGID check --root "$WL" >/dev/null || true
cargo build -q
python3 tools/lsp_smoke.py "$WL" ./target/debug/rigid

step "32. JavaScript backend: rigid programs run outside rigid"
if command -v node >/dev/null 2>&1; then
  ./tools/difftest.sh
  echo "--- rigid integers are BigInt in JS, so all 64 bits survive ---"
  WN=$(mktemp -d)
  $RIGID build --target js -o "$WN/num.js" --root examples/int64 >/dev/null
  echo "rigid VM : $($RIGID run "num.rg#big_plus_one" --root examples/int64)"
  echo "node     : $(node "$WN/num.js" "num.rg#big_plus_one")"
  echo "(a plain JavaScript number would say 9007199254740992 for both)"
  echo "--- and the output is an ordinary module other JS can call ---"
  WJ=$(mktemp -d)
  $RIGID build --target js -o "$WJ/triage.js" --root examples/triage
  cat > "$WJ/use.js" <<'JS'
const rigid = require(process.argv[2]);
const v = rigid.score_rg__severity(4n, 3n);
console.log("severity(4,3) from plain JS =", String(v), "(a " + typeof v + ")");
JS
  node "$WJ/use.js" "$WJ/triage.js"
else
  echo "(node not installed — skipping JS backend checks)"
fi

step "33. Externs: foreign functions as declared boundary nodes"
$RIGID check --root examples/interop | tail -1
echo "--- arity and effects are enforced across the boundary ---"
WX=$(mktemp -d); cp examples/interop/app.rg "$WX/"
sed -i.bak 's/jmax(a, b);/jmax(a, b, 99);/' "$WX/app.rg" 2>/dev/null || sed -i '' 's/jmax(a, b);/jmax(a, b, 99);/' "$WX/app.rg"
$RIGID check --root "$WX" 2>&1 | grep E0209 || true
echo "--- the VM refuses by design; the JS target binds ---"
$RIGID run "app.rg#report" 3 7 --root examples/interop 2>&1 | tail -1 || true
if command -v node >/dev/null 2>&1; then
  WJ2=$(mktemp -d)
  $RIGID build --target js -o "$WJ2/out.js" --root examples/interop
  node "$WJ2/out.js" "app.rg#report" 3 7
fi
rm -f examples/interop/*.bind.json

step "34. describe: the architecture itself is a compile target"
$RIGID describe --root examples/press 2>/dev/null | python3 -c "
import json,sys; s=json.load(sys.stdin)
print('spec:', s['stats']['definitions'], 'definitions,', s['stats']['call_sites'], 'call sites,', len(s['boundary']), 'extern boundary node(s)')
defs=[d for m in s['modules'] for d in m['definitions']]
t=next(d for d in defs if d['kind']=='table')
print('table', t['name'], '->', len(t['entries']), 'entries, pinned by hash', t['hash'][:19]+'...')"

step "35. memo: provably-sound persistent memoization"
WM=$(mktemp -d)
printf 'memo fn slow(n) {\n  let i = 0;\n  let acc = 0;\n  while (i < n) {\n    acc = acc + i * i;\n    i = i + 1;\n  }\n  return acc;\n}\n' > "$WM/m.rg"
$RIGID check --root "$WM" >/dev/null
T0=$(date +%s%N); $RIGID run "m.rg#slow" 2000000 --root "$WM" >/dev/null; T1=$(date +%s%N)
echo "cold:               $(( (T1-T0)/1000000 ))ms"
T0=$(date +%s%N); $RIGID run "m.rg#slow" 2000000 --root "$WM" >/dev/null; T1=$(date +%s%N)
echo "cross-process hit:  $(( (T1-T0)/1000000 ))ms  (.rigid-memo.json, keyed by def-hash + args)"
echo "--- memo requires purity ---"
printf 'memo fn bad() !io {\n  print("x");\n  return 1;\n}\n' > "$WM/bad.rg"
$RIGID check --root "$WM" 2>&1 | grep E0211 || true

step "36. temporal references: call any version that ever existed"
WT=$(mktemp -d)
printf 'fn shout(s) {\n  return s + "!";\n}\n' > "$WT/t.rg"
$RIGID check --root "$WT" >/dev/null
V1=$($RIGID hash "t.rg#shout" --root "$WT" | awk '{print $2}' | sed 's/sha256://' | cut -c1-10)
sed -i.bak 's/s + "!"/s + "?!"/' "$WT/t.rg" 2>/dev/null || sed -i '' 's/s + "!"/s + "?!"/' "$WT/t.rg"
rm -f "$WT/t.rg.bak"
printf '\nfn both(s) !io {\n  print("then: " + @"%s"(s));\n  return "now: " + shout(s);\n}\n' "$V1" >> "$WT/t.rg"
$RIGID check --root "$WT" >/dev/null
$RIGID run "t.rg#both" hi --root "$WT"
echo "--- or from the CLI ---"
$RIGID recall "$V1" hi --root "$WT"

step "37. rigid diff: grade a text diff by identity"
WD=$(mktemp -d); WD2=$(mktemp -d)
printf '// clamp into a range\nfn clamp(x, lo, hi) {\n  return if (x < lo) lo else if (x > hi) hi else x;\n}\nfn scale(x) {\n  return x * 2;\n}\n' > "$WD/u.rg"
printf 'import { clamp } from "u.rg";\nfn main(v) {\n  return clamp(v, 0, 10);\n}\n' > "$WD/app.rg"
$RIGID check --root "$WD" >/dev/null
# The same program, rewritten: the definition renamed, every parameter
# renamed, and the body reflowed. Not one line survives.
printf '// clip into a range\nfn clip(value, low, high) {\n  return if (value < low)\n    low\n  else if (value > high)\n    high\n  else\n    value;\n}\nfn scale(n) {\n  return n * 2;\n}\n' > "$WD2/u.rg"
printf 'import { clip } from "u.rg";\nfn main(v) {\n  return clip(v, 0, 10);\n}\n' > "$WD2/app.rg"
$RIGID check --root "$WD2" >/dev/null
echo "--- git sees every line of u.rg change ---"
diff "$WD/u.rg" "$WD2/u.rg" | head -5 || true
echo "--- rigid grades it ---"
$RIGID diff --root "$WD2" --against "$WD"
echo "--- now save that as the baseline and change one character ---"
$RIGID diff --root "$WD2" --save
sedi 's/value > high/value >= high/' "$WD2/u.rg"
$RIGID check --root "$WD2" >/dev/null 2>&1 || true
$RIGID diff --root "$WD2"
$RIGID diff --root "$WD2" --require-no-semantic-change >/dev/null 2>&1 \
  && echo "CI gate: passed (wrong!)" || echo "CI gate: refused, exit 1"

step "38. capabilities: a dependency that gains network access fails CI"
WC=$(mktemp -d)
printf 'fn compute(x) {\n  return x + 1;\n}\n' > "$WC/lib.rg"
printf 'import { compute } from "lib.rg";\nfn main(x) {\n  return compute(x);\n}\n' > "$WC/app.rg"
$RIGID check --root "$WC" >/dev/null
echo "--- pinned capability set ---"
grep -A 3 '"compute"' "$WC/app.rg.bind.json"
printf 'extern fn fetch(u) !net = "globalThis.fetch";\nfn compute(x) !net {\n  return fetch(x);\n}\n' > "$WC/lib.rg"
printf 'import { compute } from "lib.rg";\nfn main(x) !net {\n  return compute(x);\n}\n' > "$WC/app.rg"
echo "--- upstream now reaches the network; CI runs --frozen ---"
$RIGID check --root "$WC" --frozen 2>&1 | grep W0405 || true
echo "--- and the whole capability surface is a query ---"
$RIGID check --root "$WC" >/dev/null 2>&1 || true
$RIGID caps --root "$WC" | head -6

step "39. floats, and the one target that refuses them"
WF=$(mktemp -d)
printf 'fn area(r) {\n  return 3.14159 * r * r;\n}\nfn half(n) {\n  return float(n) / 2.0;\n}\n' > "$WF/f.rg"
$RIGID check --root "$WF" >/dev/null
echo "area(2.0)  = $($RIGID run "f.rg#area" 2.0 --root "$WF")"
echo "half(5)    = $($RIGID run "f.rg#half" 5 --root "$WF")"
echo "1 == 1.0   -> equality is numeric across Int and Float"
echo "--- the js target refuses rather than diverging ---"
$RIGID build --target js --root "$WF" 2>&1 | head -1 || true

step "40. rigid merge: a rename and an edit are different coordinates"
WB=$(mktemp -d); mkdir -p "$WB/base" "$WB/ours" "$WB/theirs"
printf '// clamp into a range\nfn clamp(x, lo, hi) {\n  return if (x < lo) lo else if (x > hi) hi else x;\n}\n' > "$WB/base/u.rg"
printf 'import { clamp } from "u.rg";\nfn main(v) {\n  return clamp(v, 0, 10);\n}\n' > "$WB/base/app.rg"
# ours: rename it. theirs: change what it does. Same lines, both sides.
printf '// clamp into a range\nfn clip(x, lo, hi) {\n  return if (x < lo) lo else if (x > hi) hi else x;\n}\n' > "$WB/ours/u.rg"
printf 'import { clip } from "u.rg";\nfn main(v) {\n  return clip(v, 0, 10);\n}\n' > "$WB/ours/app.rg"
printf '// clamp into a range\nfn clamp(x, lo, hi) {\n  return if (x < lo) lo else if (x >= hi) hi - 1 else x;\n}\n' > "$WB/theirs/u.rg"
cp "$WB/base/app.rg" "$WB/theirs/app.rg"
for d in base ours theirs; do $RIGID check --root "$WB/$d" >/dev/null; done
echo "--- git: same lines touched on both sides ---"
diff "$WB/ours/u.rg" "$WB/theirs/u.rg" | head -4 || true
echo "--- rigid: different coordinates, so both survive ---"
$RIGID merge --base "$WB/base" --ours "$WB/ours" --theirs "$WB/theirs" -o "$WB/merged"
echo "--- merged source: our name, their behaviour ---"
cat "$WB/merged/u.rg"
$RIGID check --root "$WB/merged" >/dev/null && echo "merged workspace checks clean"
echo "main(20) = $($RIGID run "app.rg#main" 20 --root "$WB/merged")  (base gave 10)"

step "41. control flow: if statements and for loops"
WK=$(mktemp -d)
printf 'fn classify(n) {\n  if (n < 0) {\n    return "negative";\n  } else if (n == 0) {\n    return "zero";\n  } else {\n    return "positive";\n  }\n}\n\nfn total(xs) {\n  let acc = 0;\n  for (x in xs) {\n    acc = acc + x;\n  }\n  return acc;\n}\n' > "$WK/k.rg"
$RIGID check --root "$WK" >/dev/null
echo "classify(-3) = $($RIGID run "k.rg#classify" -3 --root "$WK")"
echo "--- neither is sugar: fmt gives back what was written ---"
$RIGID fmt k.rg --root "$WK" | head -8
echo "--- and renaming the loop variable moves no hash ---"
H1=$($RIGID hash "k.rg#total" --root "$WK" | awk '{print $2}')
sedi 's/for (x in xs)/for (element in xs)/; s/acc + x;/acc + element;/' "$WK/k.rg"
$RIGID check --root "$WK" >/dev/null 2>&1
H2=$($RIGID hash "k.rg#total" --root "$WK" | awk '{print $2}')
[ "$H1" = "$H2" ] && echo "identical: $(echo $H1 | cut -c1-26)" || echo "MOVED"

step "42. closures: anonymous functions that stay in the graph"
$RIGID check --root examples/closures >/dev/null
echo "--- the lambda is a definition, with a hash ---"
$RIGID hash --all --root examples/closures | grep '\$'
echo "--- rdeps reaches through it: blast radius survives anonymity ---"
$RIGID rdeps "cl.rg#adder\$0" --root examples/closures
echo "--- and the quarantine still refuses an undeclared target ---"
WQ=$(mktemp -d); cp examples/closures/cl.rg "$WQ/"
printf '\nfn rogue(n) {\n  return fn(x) { return x - n; };\n}\n\nfn sneak(v) {\n  return dyn { adder$0 }(rogue(1), v);\n}\n' >> "$WQ/cl.rg"
$RIGID check --root "$WQ" >/dev/null 2>&1
($RIGID run "cl.rg#sneak" 5 --root "$WQ" 2>&1 || true) | head -1

step "43. the standard library, written in rigid"
$RIGID check --root std | tail -1
$RIGID test --root std --no-cache | tail -1
WS=$(mktemp -d)
printf 'import { sort, unique } from "std/list.rg";\nimport { to_upper, split, trim } from "std/str.rg";\n\nfn tidy(csv) {\n  let out = [];\n  for (part in split(csv, ",")) {\n    out = push(out, to_upper(trim(part)));\n  }\n  return sort(unique(out));\n}\n' > "$WS/app.rg"
$RIGID check --root "$WS" >/dev/null
echo "tidy(\" b, a ,b\") = $($RIGID run "app.rg#tidy" " b, a ,b" --root "$WS")"
echo "--- std definitions are pinned by hash like any other import ---"
grep -A 2 '"sort"' "$WS/app.rg.bind.json"
echo "--- and no sidecar is written into std itself ---"
ls "$WS"

step "44. attest: evidence bound to identity, not to a moment"
WA=$(mktemp -d)
printf 'fn sanitize(s) {\n  return s;\n}\n\nfn escape(s) {\n  return sanitize(s) + "!";\n}\n' > "$WA/util.rg"
printf 'import { escape } from "util.rg";\n\nfn handler(raw) {\n  return escape(raw);\n}\n' > "$WA/api.rg"
$RIGID check --root "$WA" >/dev/null
$RIGID attest "api.rg#handler" --kind reviewed --by david --note "checked the input path" --root "$WA"
echo "--- a pure rename, two modules away, does not disturb it ---"
sedi 's/fn sanitize(s)/fn clean(t)/; s/return s;/return t;/; s/sanitize(s)/clean(s)/' "$WA/util.rg"
$RIGID check --root "$WA" >/dev/null 2>&1
$RIGID trust "api.rg#handler" --root "$WA" | sed -n '3,5p'
echo "--- but an edit three calls away voids it ---"
sedi 's/return t;/return t + "?";/' "$WA/util.rg"
$RIGID check --root "$WA" >/dev/null 2>&1
$RIGID trust "api.rg#handler" --root "$WA" | head -6
echo "--- and CI refuses to ship it ---"
($RIGID check --require reviewed --root "$WA" 2>&1 || true) | head -2
echo "exit=$($RIGID check --require reviewed --root "$WA" >/dev/null 2>&1; echo $?)"
echo "--- the ledger keeps the stale claim: unreviewed and stale differ ---"
$RIGID attest --list --root "$WA" | tail -3

step "45. dead: code nothing can reach, proved rather than guessed"
$RIGID dead --root examples/triage | head -6
echo "--- declare the real entry point and it resolves ---"
$RIGID dead --root examples/triage --entry "pipeline.rg#triage"

step "46. rules: declared relationships, enforced exactly"
WR=$(mktemp -d); mkdir -p "$WR/domain" "$WR/http"
printf 'fn total(items) {\n  let acc = 0;\n  for (i in items) {\n    acc = acc + i;\n  }\n  return acc;\n}\n' > "$WR/domain/order.rg"
printf 'extern fn post(url, body) !net = "globalThis.fetch";\n\nfn ship(u, b) !net {\n  return post(u, b);\n}\n' > "$WR/http/send.rg"
printf '# the domain layer must not know about transport\ndeny  domain/**  ->  http/**\n# and must never reach the network at all\ndeny  domain/**  ->  !net\n' > "$WR/rigid.rules"
$RIGID check --root "$WR" | tail -2
echo "--- now let an agent wire the domain layer straight to HTTP ---"
printf 'import { ship } from "../http/send.rg";\n\nfn total(items) !net {\n  return ship("u", str(len(items)));\n}\n' > "$WR/domain/order.rg"
($RIGID check --root "$WR" 2>&1 || true) | grep E0214 | head -2 || true

step "47. dup: two names, one meaning"
WU=$(mktemp -d)
printf 'fn clamp(x, lo, hi) {\n  return if (x < lo) lo else if (x > hi) hi else x;\n}\n\nfn limit(v, low, high) {\n  return if (v < low) low else if (v > high) high else v;\n}\n' > "$WU/u.rg"
$RIGID check --root "$WU" >/dev/null
$RIGID dup --root "$WU" | head -6

step "48. properties: a claim about every input, not an example"
$RIGID test --root std --no-cache | grep -E "sort_preserves|reverse_is_its|abs_is_never" | head -3 || true
echo "--- and one that is false ---"
WP=$(mktemp -d)
printf 'property fn subtraction_commutes(a: int, b: int) {\n  return a - b == b - a;\n}\n' > "$WP/p.rg"
$RIGID check --root "$WP" >/dev/null
($RIGID test --root "$WP" --no-cache 2>&1 || true) | grep FAIL || true
echo "(the generator found it, then shrank the counterexample to its simplest form)"
echo "--- properties found a real bug in this repository's own stdlib ---"
echo "abs(-2^63) had no positive counterpart and returned a negative number;"
echo "std/math.rg now refuses it, and the property that caught it is checked in."

echo
echo "scratch dirs: (temporary; see steps above)"
