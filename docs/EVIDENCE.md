# Evidence from other people's code

Every number in `BENCHMARK.md` comes from a corpus this project's author
wrote. That is fine for showing a mechanism works and useless for showing it
matters. These two studies measure real artifacts nobody here controls.

Both are reproducible from a clean checkout, both record their inputs, and
both state what they over- and under-count. Read the limits before the
numbers.

---

## 1. Capability drift across npm releases

**Question.** rigid records an imported definition's capability set when it
pins it and reports a widening as `W0405` — an error under `--frozen`. Is
that a usable signal or is it noise? If packages gain capabilities constantly
the check is worthless; if a patch release almost never starts touching the
network, the one that does is worth stopping the build for.

**Method.** `evidence/capdrift.py`. The sample is the dependency closure of
27 seed packages (react, express, webpack, eslint, lodash, …), chosen before
any result was seen and recorded in `evidence/capdrift-sample.json`. For each
package the last 16 published versions are downloaded and scanned; the
capability set of each version is compared against its predecessor and
classified by the semver step between them.

Only code **reachable from the package's declared entry points** is scanned —
`main`, `module`, `browser`, and the `exports` map, followed transitively
through relative imports. This matters: lodash 4.17.14 ships a `tag.js` that
requires `child_process`, but nothing a consumer imports ever loads it.
Counting it reported a capability that never runs. Install hooks are the
exception and always count, because they run whether anything imports them
or not.

**Result** — 299 packages, 3,832 versions, 3,533 consecutive upgrades:

| step  | upgrades | widened | rate |
|-------|---------:|--------:|-----:|
| patch |    2,056 |      11 | **0.5%** |
| minor |      942 |      15 | 1.6% |
| major |      535 |      36 | 6.7% |

42 of 299 packages widened at least once.

**Reading.** A patch release almost never starts reaching a new part of the
host — about one upgrade in 190. That is the property the mechanism needs:
`W0405` would stay quiet through the ordinary flow of dependency updates, so
when it does fire it carries information. A check that fired on a fifth of
patch upgrades would be turned off within a week.

The capabilities gained on a patch, across the whole sample, were `env` ×6,
`rand` ×4, `time` ×4, `fs` ×3, `net` ×1, `proc` ×1. Seventeen of those are
ambient (`env`, `rand`, `time`); five are the ones that matter.

**What this does not show.** It does not show that `W0405` catches attacks —
every documented npm compromise was unpublished from the registry, so the
malicious artifacts cannot be fetched and measured. It shows the *base rate*
against which such an event would stand out, which is the prerequisite claim
and the one that was actually testable.

**Limits.** The scanner recovers capabilities by matching `require`/`import`
of host modules and the global forms (`fetch`, `eval`, `process.env`, …).
That over-counts — a string in a comment matches — and under-counts — a fully
dynamic `require(x)` does not. rigid's own capability sets are exact because
they are *declared* on `extern` and propagated along a graph the compiler
built; this is a proxy for what those declarations would say. Minified
bundles are skipped. A package whose entry points cannot be resolved falls
back to scanning everything, which over-counts for that package.

```
python3 evidence/capdrift.py --packages 300 --versions 16
python3 evidence/capdrift.py --report
```

---

## 2. Renames that shipped a dangling reference

**Question.** rigid's founding claim is that a name-bound language lets a
rename silently break a reference. How often does that actually happen in
code people ship?

**Method.** `evidence/renamebreak.py`. Walk a repository's history commit by
commit. At each commit, parse every changed `.py` file with Python's own
`ast` and record its top-level `def`/`class` names. A name that disappears
from a file while exactly one same-kind, same-arity name appears in the same
file is treated as a rename. Then: immediately after that commit, does any
*other* file in the tree still reference the old name in a call-shaped or
import-shaped position?

If it does, the rename shipped an incomplete edit — the state rigid refuses
to produce, because the pinned identity is found under the new name and the
reference is reported with a mechanical `rename_reference` fix.

Ten Python projects, chosen before any result was seen and listed in
`evidence/repos.txt`: requests, flask, click, black, httpx, attrs, fastapi,
scrapy, more-itertools, pytest. None written by this author.

A candidate only counts as a rename if the old name is gone from the **whole
tree** at that commit, not merely from the file it left. Without that check,
`batched` in more-itertools reads as renamed to `constrained_batches` while
`batched` is still a public function, and every generic `test_it` in pytest
reads as renamed to whatever else the commit added. Adding it cut the finding
count from 37 to 8.

**Result** — 22,583 commits across ten repositories:

| repository | renames | dangling | rate |
|---|---:|---:|---:|
| requests | 8 | 0 | 0.0% |
| flask | 46 | 0 | 0.0% |
| click | 53 | 0 | 0.0% |
| black | 31 | 1 | 3.2% |
| httpx | 74 | 4 | 5.4% |
| attrs | 25 | 1 | 4.0% |
| fastapi | 11 | 0 | 0.0% |
| scrapy | 113 | 0 | 0.0% |
| more-itertools | 34 | 1 | 2.9% |
| pytest | 63 | 1 | 1.6% |
| **TOTAL** | **458** | **8** | **1.7%** |

All eight, in full:

```
black          FileMode        -> Mode             still in blackd.py, tests/test_black.py
httpx          Client          -> AsyncClient      still in httpx/__init__.py
httpx          Event           -> Lock             still in tests/conftest.py
httpx          Event           -> Lock             still in tests/conftest.py
httpx          encode          -> encode_request   still in httpx/_auth.py, httpx/_client.py
attrs          attr            -> attrib           still in conftest.py, tests/test_annotations.py
more-itertools unique_from_each -> unique_to_each  still in more_itertools/tests/test_more.py
pytest         CallSpec2       -> CallSpec         still in testing/deprecated_test.py
```

**Verified by hand.** The black case: commit `06f2790b5ca3`, titled *"Rename
FileMode into just Mode"*. `FileMode` is defined nowhere in the tree
afterwards, `Mode` is the new class, and `blackd.py:109` still reads
`mode = black.FileMode(`. That is precisely the state rigid refuses to
produce — the pinned identity is found under the new name, and the reference
is reported as `E0201` carrying a mechanical `rename_reference` fix.

**Reading, without inflation.** 1.7% is a *low* rate, and the population
explains most of it: these are ten of the best-maintained projects in the
Python ecosystem, with CI, high coverage, and careful review. The honest
claim is not "renames break things constantly" — it is that renames leave
dangling references even here, roughly once per sixty, and that every one of
the eight is a case a content-addressed compiler turns into an error at the
moment of the rename rather than a bug someone finds later.

Whether the rate is higher in less-tested code is a reasonable guess and is
**not** measured here.

**A correction worth recording.** The first version of this study reported
`0 dangling` across 379 renames. That number was an artifact: the reference
search used `git grep -E`, and git's ERE engine does not implement `\b` — it
matched nothing, silently, on every query. A confident zero from a broken
instrument is worse than no result, so the script now runs positive and
negative controls against the first repository before measuring anything and
refuses to continue if the detector finds nothing where a plain fixed-string
search finds something.

**Limits, which are substantial.** Python is dynamic, so a surviving textual
reference is not proof of a runtime break: it may be shadowed, guarded, in
dead code, or a coincidental match. Matches are restricted to call- and
import-shaped positions and short or dunder names are dropped, and it still
over-counts. Renames are inferred from the shape of a diff rather than from
intent, so a delete plus an unrelated add in one commit can look like one.
Only the repository's own files are searched — a break in a *dependent*
project is invisible here, and that is the larger population.

The output is a rate of **plausible** dangling references, not of confirmed
production incidents.

```
python3 evidence/renamebreak.py --repos evidence/repos.txt --commits 2500
```

---

## Why these two

They test different halves of the thesis. The first asks whether a mechanism
rigid has built is *usable* — a signal is only worth having if it is quiet.
The second asks whether the problem rigid exists to solve is *real* outside
the benchmark that was written to demonstrate it.

Neither is a proof that anyone should adopt this language. They are the
smallest honest step past "the author's own tests pass."
