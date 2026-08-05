# Dispatch-width study: do declared target sets stay narrow?

Corpus and method were registered before any repository was fetched; see
PREREGISTRATION.md. Two protocol deviations are recorded there, including
one discarded run.

## Hypothesis
H1: in real string-dispatched code, dispatch sets are narrow — median
width <= 4, and most sites are singletons.
H0 (falsifier): median width > 10, or most dispatch sites wide (>10).

## Corpus
Five widely-used Python projects with plugin/registry architectures,
named in advance: flask, scrapy, celery, pygments, httpie.
983 non-test source files parsed.

## Result 1 — width distribution (430 dispatch tables/registries pooled)

| statistic | targets |
|---|---|
| median | 1 |
| p75 | 3 |
| p90 | 5 |
| p95 | 9 |
| p99 | 35 |
| max | 602 (pygments lexer registry) |

**88.8%** of dispatch tables have <= 4 targets. **3.0%** have > 10.

Per project (median / p90 / max):
flask 2 / 3 / 3 · scrapy 1 / 3 / 6 · celery 1 / 5 / 220 ·
pygments 9 / 50 / 602 · httpie 2 / 5 / 6

**H1 is supported; H0 is not met.** The distribution is sharply
right-skewed: a large majority of dispatch is narrow, with a thin tail of
very large registries concentrated in projects whose *purpose* is a
catalogue (pygments' lexers, celery's task/state maps).

## Result 2 — how much dispatch is statically knowable at all
Pooled non-method call sites: 25,980 direct, 543 unbounded
(runtime-string imports: `import_string`, `load_object`, computed
`getattr`). **2.05%** of non-method call sites are unbounded in the
original source — small in count, but these are exactly the sites where
"who calls this?" is unanswerable today, and they sit at the
architectural seams (plugin loading, middleware chains, entry points).

## Result 3 — ground truth cross-check
httpie's formatter and auth plugin registries (`plugins/registry.py`,
`output/processing.py`) were migrated by hand to rigid. The analyzer
predicted widths of 4 and 3; `rigid exactness` on the migrated module
reports:

    call sites: 8 · singleton: 6 · table sites: 2 (widths 4, 3)
    exactness: 0.750

Analyzer and language agree on the widths. In the original, the same
question ("which formatters can run here?") has no static answer: the
registry is populated at import time and iterated dynamically. After
migration, `rdeps json_formatter` answers it exactly.

## Honest limits
- Five projects, one language (Python). Not a random sample; chosen by
  architecture type, though fixed before any measurement.
- The analyzer is conservative and matches only the pre-declared
  patterns; dispatch it cannot see is not counted (coverage is reported
  as raw counts, not claimed as complete).
- Only one cluster was hand-migrated; a full migration could split or
  merge sites and shift widths.
- Python method calls (26,575 pooled) are receiver-polymorphic and were
  deliberately excluded from the exactness denominator rather than
  counted as singletons — the error that invalidated run 1. rigid has no
  methods, so there is no clean mapping for them; this is the largest
  threat to external validity.
- Run by the language's author. The pre-registration constrains that; it
  does not eliminate it.

## What this licenses the paper to say
That declared-set width in real dispatch-heavy code is narrow at the
median (1) and at p90 (5), with a thin heavy tail; that the tail is
concentrated in catalogue-shaped registries where a wide set is honest
rather than a modelling failure; and that the exactness metric agrees
with hand migration on a real cluster. It does not license claims about
whole-system migration cost, non-Python ecosystems, or developer
behaviour over time.
