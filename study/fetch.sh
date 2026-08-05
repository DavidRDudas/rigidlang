#!/usr/bin/env bash
# Downloads the five pre-registered corpora into study/repos/
# Portable: no GNU-only flags (works on macOS and Linux).
cd "$(dirname "$0")" || exit 1
mkdir -p repos && cd repos || exit 1

for r in pallets/flask scrapy/scrapy celery/celery pygments/pygments httpie/httpie; do
  n=${r#*/}
  got=""
  for branch in main master; do
    curl -sL "https://codeload.github.com/$r/tar.gz/refs/heads/$branch" -o "$n.tgz"
    if tar tzf "$n.tgz" >/dev/null 2>&1; then got=$branch; break; fi
  done
  if [ -z "$got" ]; then
    echo "$n: FAILED to download (no main/master tarball)"
    continue
  fi
  rm -rf "$n" && mkdir -p "$n"
  tar xzf "$n.tgz" -C "$n" --strip-components=1
  echo "$n: $(find "$n" -name '*.py' | wc -l | tr -d ' ') py files (branch: $got)"
done
