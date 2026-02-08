#!/usr/bin/env bash
set -euo pipefail

# Run Kagzi examples sequentially. Skips worker_hub (long-running).
server="${1:-http://localhost:50051}"
export KAGZI_SERVER_URL="$server"

examples=(
  01_basics
  02_error_handling
  03_scheduling
  04_concurrency
  05_fan_out_in
  06_long_running
  07_idempotency
  08_saga_pattern
  09_data_pipeline
  10_multi_queue
)

for ex in "${examples[@]}"; do
  printf "\n=== Running %s against %s ===\n" "$ex" "$KAGZI_SERVER_URL"
  cargo run -p examples --example "$ex"
done
