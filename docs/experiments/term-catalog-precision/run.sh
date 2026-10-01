#!/usr/bin/env bash
# Term catalog precision experiment.
#   ./run.sh collect               mine raw candidates, process, write the private labeling sheet
#   ./run.sh collect labels SHEET  import a filled labeling sheet as raw label files
#   ./run.sh process               aggregate raw files into data/processed and the private folder
#   ./run.sh analyze               process, then write results/ from raw files
#   ./run.sh verify                check SHA-256 of public and private raw files
#   ./run.sh all                   verify and analyze
set -euo pipefail

cd "$(dirname "$0")"
export PYTHONDONTWRITEBYTECODE=1
export TERM_CATALOG_PRIVATE_DIR="${TERM_CATALOG_PRIVATE_DIR:-$HOME/workspace/woon/.local/orchestration/saturn-experiments/term-catalog-precision/raw}"

write_sums() {
  (cd data && shasum -a 256 raw/*.jsonl > SHA256SUMS)
  (cd "$TERM_CATALOG_PRIVATE_DIR" && shasum -a 256 cooc-*.jsonl collect-log-*.json $(ls labels-cooc-*.jsonl 2>/dev/null)) \
    | awk '{print $1 "  " $2}' > data/private-SHA256SUMS
}

verify() {
  (cd data && shasum -a 256 -c SHA256SUMS)
  (cd "$TERM_CATALOG_PRIVATE_DIR" && shasum -a 256 -c "$OLDPWD/data/private-SHA256SUMS")
}

case "${1:-}" in
  collect)
    if [ "${2:-}" = "labels" ]; then
      python3 scripts/01-collect.py labels "$3"
    else
      python3 scripts/01-collect.py mine
      python3 scripts/02-process.py
    fi
    write_sums
    ;;
  process) python3 scripts/02-process.py ;;
  analyze)
    python3 scripts/02-process.py
    python3 scripts/03-analyze.py
    ;;
  verify) verify ;;
  all)
    verify
    python3 scripts/02-process.py
    python3 scripts/03-analyze.py
    ;;
  *)
    sed -n '2,8p' "$0"
    exit 2
    ;;
esac
