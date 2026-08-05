#!/usr/bin/env bash
# Differential test: rigid's VM vs the emitted JavaScript.
# Every case runs both engines and compares the last line of output.
RB=${RB:-./target/release/rigid}
OUT=$(mktemp -d)
pass=0; fail=0
build() { $RB build --target js -o "$OUT/$1.js" --root "examples/$1" >/dev/null 2>&1 || true; }
chk() {
  local name="$1" ex="$2" ref="$3"; shift 3
  local a b
  a=$($RB run "$ref" "$@" --root "examples/$ex" 2>&1 | tail -1)
  b=$(node "$OUT/$ex.js" "$ref" "$@" 2>&1 | tail -1)
  if [ "$a" = "$b" ]; then
    printf 'MATCH  %-22s %s\n' "$name" "$a"; pass=$((pass+1))
  else
    printf 'DIFF   %s\n   rigid: %s\n   node : %s\n' "$name" "$a" "$b"; fail=$((fail+1))
  fi
}
for e in flow data std triage hello httpie-migration mandel int64 floats control closures; do build $e; done
chk "iteration"      flow  "flow.rg#fib_iter" 30
chk "mutation"       flow  "flow.rg#sum_to" 100
chk "try/catch"      flow  "flow.rg#safe_div" 10 0
chk "uncaught fail"  flow  "flow.rg#checked_div" 5 0
chk "records"        data  "data.rg#make_incident" network 7
chk "lists+recursion" data "data.rg#build" 5 "[]"
chk "arity error"    data  "data.rg#sum" 0
chk "chars/join"     std   "std.rg#slug" abc
chk "str/substr"     std   "std.rg#report" 7
chk "catchable int"  std   "std.rg#parse_or" nope -1
chk "table dispatch" triage "pipeline.rg#triage" network 3 3
chk "table miss"     triage "pipeline.rg#triage" hardware 3 3
chk "dyn dispatch"   hello "dispatch.rg#apply" 2 10
chk "quarantine"     hello "dispatch.rg#escape" 10
chk "migrated table" httpie-migration "plugins.rg#authenticate" bearer tok
chk "fixed-point"    mandel "mandel.rg#iterate" 0 0 0 0 0 28
# 64-bit integers and floats: a JavaScript number is one float64, so both of
# these are only equal if the backend stopped using it for everything.
chk "int64 literal"  int64  "num.rg#big"
chk "int64 add"      int64  "num.rg#big_plus_one"
chk "int64 product"  int64  "num.rg#wide_product"
chk "int division"   floats "f.rg#int_div"
chk "float division" floats "f.rg#float_div"
chk "float math"     floats "f.rg#area" 2.0
chk "float convert"  floats "f.rg#half" 5
chk "float shown"    floats "f.rg#shown"
chk "cross equality" floats "f.rg#cross_eq"
chk "if statement"   control "c.rg#classify" -3
chk "else if chain"  control "c.rg#classify" 0
chk "for over list"  control "c.rg#sum_upto" 4
chk "for over string" control "c.rg#spell" abc
chk "if inside loop" control "c.rg#evens_upto" 10
chk "closure capture" closures "cl.rg#add_then_sum" 10 4
chk "closure scale"   closures "cl.rg#scale_then_sum" 3
chk "two captures"    closures "cl.rg#two_captures" 5 2
echo "=== $pass matched, $fail differed ==="
[ "$fail" -eq 0 ]
