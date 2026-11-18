#!/usr/bin/env bash
# Checks the backend's test coverage against the thresholds CI holds, from
# the summary cargo-llvm-cov writes (just coverage-backend):
#
#   - all crates together: at least $total_min % of lines
#   - every crate: at least $crate_min %, but logdockd, whose rest is the
#     glue of the binary (signals, logging)
#   - the domain layers together: at least $domain_min %
#
# The thresholds sit just below what the tests reach: coverage may rise,
# but not fall unnoticed. Prints the coverage of every crate.
#
# Usage: scripts/coverage-check.sh REPORT.json
set -euo pipefail

report=${1:?usage: $0 REPORT.json}
total_min=85
crate_min=75
domain_min=90
glue=(logdockd)

summary=$(jq -r '
  .data[0] as $d
  | ($d.files | map({
      crate: (.filename | capture("/crates/(?<c>[^/]+)/").c),
      domain: (.filename | test("/src/domain(\\.rs|/)")),
      count: .summary.lines.count,
      covered: .summary.lines.covered
    })) as $files
  | (["total", $d.totals.lines.count, $d.totals.lines.covered] | @tsv),
    ($files | map(select(.domain)) | ["domain", (map(.count) | add // 0), (map(.covered) | add // 0)] | @tsv),
    ($files | group_by(.crate)[] | [.[0].crate, (map(.count) | add), (map(.covered) | add)] | @tsv)
' "$report")

# The percentage of a line in the summary: name, lines, covered.
percent() { awk -F'\t' -v name="$1" '$1 == name { printf "%.1f", ($2 ? 100 * $3 / $2 : 100) }' <<<"$summary"; }
below() { awk -v value="$1" -v min="$2" 'BEGIN { exit !(value < min) }'; }

printf '%-28s %8s %8s\n' crate lines covered
awk -F'\t' '$1 ~ /^logdock/ { printf "%-28s %8d %7.1f%%\n", $1, $2, ($2 ? 100 * $3 / $2 : 100) }' <<<"$summary"

failed=()
total=$(percent total)
domain=$(percent domain)
printf '%-28s %17s%%\n%-28s %17s%%\n' "all crates" "$total" "domain layers" "$domain"
below "$total" "$total_min" && failed+=("all crates: $total % < $total_min %")
below "$domain" "$domain_min" && failed+=("domain layers: $domain % < $domain_min %")
while IFS=$'\t' read -r crate count covered; do
    [[ $crate == logdock* ]] || continue
    [[ " ${glue[*]} " == *" $crate "* ]] && continue
    value=$(awk -v c="$covered" -v n="$count" 'BEGIN { printf "%.1f", (n ? 100 * c / n : 100) }')
    below "$value" "$crate_min" && failed+=("$crate: $value % < $crate_min %")
done <<<"$summary"

if ((${#failed[@]})); then
    printf 'coverage below the thresholds:\n' >&2
    printf '  %s\n' "${failed[@]}" >&2
    exit 1
fi
