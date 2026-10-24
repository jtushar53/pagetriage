#!/usr/bin/env bash
# Append one line of public usage counts to a CSV: GitHub stars, forks, watchers, 14-day views and
# unique visitors, 14-day clones and unique cloners, and (once published) crates.io, PyPI and npm
# downloads. GitHub keeps traffic for only 14 days, so a weekly snapshot is the only way to keep a
# history. Nothing here identifies a person: registries and GitHub publish counts only.
#
#   scripts/stats-snapshot.sh [stats.csv]        (needs `gh` logged in as the owner for traffic)
set -euo pipefail
OUT=${1:-stats.csv}
REPO=jtushar53/pagetriage
date=$(date -u +%F)

j() { gh api "$1" --jq "$2" 2>/dev/null || echo ""; }
stars=$(j "repos/$REPO" .stargazers_count)
forks=$(j "repos/$REPO" .forks_count)
watchers=$(j "repos/$REPO" .subscribers_count)
views=$(j "repos/$REPO/traffic/views" .count)
visitors=$(j "repos/$REPO/traffic/views" .uniques)
clones=$(j "repos/$REPO/traffic/clones" .count)
cloners=$(j "repos/$REPO/traffic/clones" .uniques)

# Registries: empty until the packages exist.
get() { curl -fsSL --max-time 20 "$1" 2>/dev/null || true; }
crates=$(get "https://crates.io/api/v1/crates/pagetriage-core" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["crate"]["downloads"])' 2>/dev/null || echo "")
pypi=$(get "https://pypistats.org/api/packages/pagetriage/recent" | python3 -c 'import sys,json; print(json.load(sys.stdin)["data"]["last_month"])' 2>/dev/null || echo "")
npm=$(get "https://api.npmjs.org/downloads/point/last-month/pagetriage" | python3 -c 'import sys,json; print(json.load(sys.stdin)["downloads"])' 2>/dev/null || echo "")

[ -s "$OUT" ] || echo "date,stars,forks,watchers,views_14d,visitors_14d,clones_14d,cloners_14d,crates_total,pypi_month,npm_month" > "$OUT"
echo "$date,$stars,$forks,$watchers,$views,$visitors,$clones,$cloners,$crates,$pypi,$npm" >> "$OUT"
tail -1 "$OUT"
