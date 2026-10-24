#!/usr/bin/env bash
# Append one line of public usage counts to a CSV: GitHub stars, forks, watchers, 14-day views and
# unique visitors, 14-day clones and unique cloners, and (once published) crates.io, PyPI and npm
# downloads. GitHub keeps traffic for only 14 days, so a weekly snapshot is the only way to keep a
# history. Nothing here identifies a person: registries and GitHub publish counts only.
#
#   scripts/stats-snapshot.sh [stats.csv]
#
# The traffic endpoints (views, clones) need a token with push access to the repository. The
# workflow's own token does not have it ("Resource not accessible by integration"), so in Actions
# those four columns stay empty unless the owner adds a repository secret STATS_TOKEN holding a
# fine-grained personal access token for this repository with "Administration: read" permission.
# Locally, `gh` logged in as the owner is enough. A value that is not a plain number is recorded as
# empty rather than written into the CSV.
set -euo pipefail
OUT=${1:-stats.csv}
REPO=jtushar53/pagetriage
date=$(date -u +%F)

j() {
  local v
  v=$(gh api "$1" --jq "$2" 2>/dev/null) || v=""
  [[ $v =~ ^[0-9]+$ ]] && echo "$v" || echo ""
}
stars=$(j "repos/$REPO" .stargazers_count)
forks=$(j "repos/$REPO" .forks_count)
watchers=$(j "repos/$REPO" .subscribers_count)
views=$(j "repos/$REPO/traffic/views" .count)
visitors=$(j "repos/$REPO/traffic/views" .uniques)
clones=$(j "repos/$REPO/traffic/clones" .count)
cloners=$(j "repos/$REPO/traffic/clones" .uniques)

# Registries: empty until the packages exist.
get() { curl -fsSL --max-time 20 "$1" 2>/dev/null || true; }
num() { python3 -c "import sys,json; v=json.load(sys.stdin)$1; print(v if isinstance(v,int) else '')" 2>/dev/null || echo ""; }
crates=$(get "https://crates.io/api/v1/crates/pagetriage-core" | num '["crate"]["downloads"]')
pypi=$(get "https://pypistats.org/api/packages/pagetriage/recent" | num '["data"]["last_month"]')
npm=$(get "https://api.npmjs.org/downloads/point/last-month/pagetriage" | num '["downloads"]')

[ -s "$OUT" ] || echo "date,stars,forks,watchers,views_14d,visitors_14d,clones_14d,cloners_14d,crates_total,pypi_month,npm_month" > "$OUT"
echo "$date,$stars,$forks,$watchers,$views,$visitors,$clones,$cloners,$crates,$pypi,$npm" >> "$OUT"
tail -1 "$OUT"
