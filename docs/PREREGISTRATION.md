# Exactness study — pre-registration
# Written BEFORE any repository was fetched or any number computed.
# Purpose: fix the selection rule, the patterns, and the metric in advance
# so results cannot be tuned to favour the hypothesis.

## Hypothesis (H1)
In real string-dispatched code, the set of possible targets at a dispatch
site is NARROW: median declared-set width <= 4, and >= 60% of all call
sites are singletons (reference exactness E >= 0.60).

## Falsifier (H0)
If median dispatch width > 10, or a majority of dispatch sites are wide
(>10 targets), the quarantine is "technically exact, practically
useless" and the thesis is in trouble. This outcome WILL be reported.

## Selection rule (fixed in advance)
- Source: GitHub API search, language:Python, sorted by stars, descending.
- Query terms chosen to find registry/table dispatch, not chosen by me
  after inspection: repositories matching a code-agnostic topic search.
- Take the first N=5 repositories that (a) are libraries or applications,
  not tutorials/awesome-lists/books, and (b) contain at least one
  dict-literal or registry dispatch site detected by the analyzer.
- No repository is skipped after its numbers are known. If a repo is
  excluded, the reason is recorded here before its numbers are read.

## What is measured
For each detected dispatch site:
  width = number of distinct callable targets reachable at that site.
Call sites are classified:
  - singleton  : a direct call to one statically-known function (width 1)
  - dispatch   : a call through a string-keyed table/registry (width n)
  - unknown    : an indirect call whose target set cannot be bounded
                 statically (getattr with a computed name, eval, etc.)
Reference exactness  E = singleton_sites / (singleton + dispatch + unknown).
Reported: E, the full width distribution, the count of unknown sites, and
the fraction of dispatch that was statically knowable at all.

## Patterns the analyzer will detect (conservative, declared now)
P1  dict literal mapping str -> function name, later indexed and called
P2  decorator registry: @x.register("name") / @x.route("...") accumulating
    into one dict, then dispatched
P3  getattr(obj, literal_prefix + var)  -> bounded by methods with prefix
P4  getattr/eval with a fully computed name -> UNKNOWN (unbounded)
Anything not matching P1-P4 is not counted as a dispatch site; coverage
(sites matched / total call sites) is reported so the reader can judge.

## Ground truth cross-check
One detected dispatch cluster is migrated by hand into rigid, and
`rigid exactness` is run on it. If the analyzer's width and rigid's
measured width disagree, the analyzer is wrong and its numbers are
reported with that caveat.

## Threats to validity to be stated in the writeup
- Python != rigid: a migration may split or merge sites.
- The analyzer is conservative: it under-counts dispatch it cannot see.
- Author bias: the study is run by the language's author (an AI model);
  the pre-registration exists to constrain that, not to eliminate it.

## Protocol deviation #1 (recorded before any data was seen)
GitHub's search API returned 0 remaining quota for unauthenticated use,
so star-ranked selection was impossible. Substituted rule: a fixed list
of five widely-used Python projects with plugin/registry architectures,
written down here BEFORE any repository was downloaded or analyzed:

  1. pallets/flask
  2. scrapy/scrapy
  3. celery/celery
  4. pygments/pygments
  5. httpie/httpie

Selection was by architecture type (registry/plugin dispatch), not by any
knowledge of their width distributions, which I do not have. The list is
frozen: every repo on it is analyzed and reported, including any whose
numbers are unfavourable.

## Protocol deviation #2 (recorded on discovery, before results were written up)
The first analyzer counted every attribute call `x.foo()` as a singleton
site. In Python that is receiver-polymorphic dispatch, not a static call,
and counting it as singleton inflates E toward 1.0 for trivial reasons.
Run 1 is DISCARDED (its numbers: E=0.977-0.998, reported here so the
discard is auditable, not hidden). Analyzer v2 classifies:
  direct   : call to a module-level function name
  method   : attribute call (receiver-polymorphic; NOT singleton)
  table    : call through a str-keyed dict/registry (width = entries)
  dynimport: target named by a runtime string path (UNBOUNDED)
and reports registry-size distribution separately, since registry size is
what a rigid `table` would actually contain after migration.
