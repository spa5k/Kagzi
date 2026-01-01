#!/bin/bash

# Test script for running all Kagzi examples
# This script runs each example and reports the results

set -e

echo "======================================"
echo "Kagzi Examples Test Suite"
echo "======================================"
echo ""

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test counter
TOTAL=0
PASSED=0
FAILED=0

# Function to run a test
run_test() {
    local example=$1
    local variant=$2
    local timeout=${3:-10}
    
    TOTAL=$((TOTAL + 1))
    echo -e "${YELLOW}[$TOTAL] Testing: $example${variant:+ ($variant)}${NC}"
    
    if [ -z "$variant" ]; then
        CMD="cargo run -p examples --example $example 2>&1"
    else
        CMD="cargo run -p examples --example $example -- $variant 2>&1"
    fi
    
    # Run with timeout
    if timeout ${timeout}s bash -c "$CMD" > /tmp/kagzi_test_${example}_${variant}.log 2>&1; then
        echo -e "${GREEN}✓ PASSED${NC}"
        PASSED=$((PASSED + 1))
        return 0
    else
        EXIT_CODE=$?
        if [ $EXIT_CODE -eq 124 ]; then
            echo -e "${RED}✗ FAILED (timeout after ${timeout}s)${NC}"
        else
            echo -e "${RED}✗ FAILED (exit code: $EXIT_CODE)${NC}"
        fi
        echo "  Log: /tmp/kagzi_test_${example}_${variant}.log"
        FAILED=$((FAILED + 1))
        return 1
    fi
}

echo "Starting tests..."
echo ""

# 01_basics - has multiple variants
echo "=== 01_basics ==="
run_test "01_basics" "hello" 10
run_test "01_basics" "chain" 10
run_test "01_basics" "context" 10
run_test "01_basics" "sleep" 30
echo ""

# 02_error_handling
echo "=== 02_error_handling ==="
run_test "02_error_handling" "" 10
echo ""

# 03_scheduling
echo "=== 03_scheduling ==="
run_test "03_scheduling" "" 15
echo ""

# 04_concurrency
echo "=== 04_concurrency ==="
run_test "04_concurrency" "" 15
echo ""

# 05_fan_out_in
echo "=== 05_fan_out_in ==="
run_test "05_fan_out_in" "" 15
echo ""

# 06_long_running
echo "=== 06_long_running ==="
run_test "06_long_running" "" 20
echo ""

# 07_idempotency
echo "=== 07_idempotency ==="
run_test "07_idempotency" "" 15
echo ""

# 08_saga_pattern
echo "=== 08_saga_pattern ==="
run_test "08_saga_pattern" "" 15
echo ""

# 09_data_pipeline
echo "=== 09_data_pipeline ==="
run_test "09_data_pipeline" "" 20
echo ""

# 10_multi_queue
echo "=== 10_multi_queue ==="
run_test "10_multi_queue" "" 15
echo ""

# Summary
echo "======================================"
echo "Test Summary"
echo "======================================"
echo -e "Total:  $TOTAL"
echo -e "${GREEN}Passed: $PASSED${NC}"
echo -e "${RED}Failed: $FAILED${NC}"
echo ""

if [ $FAILED -eq 0 ]; then
    echo -e "${GREEN}All tests passed! 🎉${NC}"
    exit 0
else
    echo -e "${RED}Some tests failed. Check logs in /tmp/kagzi_test_*.log${NC}"
    exit 1
fi
