# Parallel Execution Testing Guide

This document describes the comprehensive test suite for Week 5 of the parallel execution implementation.

## Overview

The Week 5 testing covers:
- **Edge Cases**: Race conditions, partial failures, nested parallelism
- **Integration Tests**: Comprehensive end-to-end scenarios
- **Performance Benchmarks**: Measuring speedup and overhead

## Test Suite Organization

### Edge Case Tests (`parallel_edge_cases_test.rs`)

Located at: `crates/kagzi/tests/parallel_edge_cases_test.rs`

#### Test 1: Race Condition on Cache Insert
**Purpose**: Verify that when multiple parallel steps try to cache the same result simultaneously, the ON CONFLICT handling works correctly.

**What it tests**:
- Multiple steps executing in parallel don't duplicate work
- Database ON CONFLICT clause prevents race conditions
- Memoization cache is thread-safe

**Expected behavior**: Each step executes exactly once, even with concurrent access.

#### Test 2: Partial Failure with FailFast Strategy
**Purpose**: Verify that when some parallel steps succeed and others fail, the FailFast strategy stops immediately.

**What it tests**:
- Error propagation in parallel execution
- FailFast strategy implementation
- Workflow state after partial failure

**Expected behavior**: Workflow fails immediately when any step fails, aborting remaining steps.

#### Test 3: Nested Parallelism
**Purpose**: Test parallel execution inside parallel execution (nested parallelism).

**What it tests**:
- Outer parallel group executes correctly
- Inner parallel group within a parallel step works
- Parent-child step relationship tracking

**Expected behavior**: Both outer and inner parallel groups complete successfully.

#### Test 4: Parallel with Retry
**Purpose**: Verify that retry policies work correctly with parallel steps.

**What it tests**:
- Retry policy integration with parallel execution
- Exponential backoff in parallel context
- Attempt counting and retry scheduling

**Expected behavior**: Failed parallel step retries automatically according to policy.

#### Test 5: Memoization Across Multiple Workflow Runs
**Purpose**: Verify that parallel steps are properly memoized across workflow restarts.

**What it tests**:
- Cache persistence across workflow runs
- Step execution counting
- Memoization effectiveness

**Expected behavior**: Second execution uses cached results without re-executing steps.

#### Test 6: Race() with All Failures
**Purpose**: Test race() when all steps fail.

**What it tests**:
- Race behavior when no step succeeds
- Error collection and reporting
- Graceful handling of complete failure

**Expected behavior**: Race fails with meaningful error message listing all failures.

#### Test 7: Large-Scale Parallel Execution
**Purpose**: Test scalability with many parallel steps (50 steps).

**What it tests**:
- Performance with high step count
- Database connection pooling
- Memory efficiency
- Result ordering preservation

**Expected behavior**: All 50 steps complete successfully with correct result ordering.

## Running Tests

### Prerequisites

1. PostgreSQL database running
2. Database URL configured:
   ```bash
   export DATABASE_URL="postgres://postgres:postgres@localhost:5432/kagzi_test"
   ```
3. Run migrations:
   ```bash
   cd /path/to/Kagzi
   sqlx migrate run --database-url $DATABASE_URL
   ```

### Running All Tests

```bash
cargo test --package kagzi --test parallel_edge_cases_test -- --ignored
```

### Running Individual Tests

```bash
# Race condition test
cargo test --package kagzi --test parallel_edge_cases_test test_race_condition_cache_insert -- --ignored --nocapture

# Partial failure test
cargo test --package kagzi --test parallel_edge_cases_test test_partial_failure_fail_fast -- --ignored --nocapture

# Nested parallelism test
cargo test --package kagzi --test parallel_edge_cases_test test_nested_parallelism -- --ignored --nocapture

# Parallel with retry test
cargo test --package kagzi --test parallel_edge_cases_test test_parallel_with_retry -- --ignored --nocapture

# Memoization test
cargo test --package kagzi --test parallel_edge_cases_test test_parallel_memoization_across_runs -- --ignored --nocapture

# Race all failures test
cargo test --package kagzi --test parallel_edge_cases_test test_race_all_failures -- --ignored --nocapture

# Large-scale test
cargo test --package kagzi --test parallel_edge_cases_test test_large_scale_parallel -- --ignored --nocapture
```

## Performance Benchmarks (`parallel_performance.rs`)

Located at: `crates/kagzi/benches/parallel_performance.rs`

### Benchmark 1: Parallel vs Sequential Execution
**Purpose**: Measure speedup gained from parallel execution vs sequential execution.

**Measures**:
- Execution time for 3, 5, 10, and 20 steps
- Throughput (steps/second)
- Speedup factor

**Expected results**:
- For I/O-bound operations (50ms each):
  - 3 steps: ~3x speedup
  - 10 steps: ~10x speedup
- Diminishing returns with very high step counts due to coordination overhead

### Benchmark 2: Memoization Cache Performance
**Purpose**: Measure the performance benefit of memoization cache.

**Measures**:
- First run (cold cache) vs second run (warm cache)
- Cache lookup time
- Speedup from cached results

**Expected results**:
- Cached execution should be 10-100x faster than uncached
- Cache overhead should be <5ms per step

### Benchmark 3: Race() Performance
**Purpose**: Measure race() performance with varying step counts.

**Measures**:
- Time to return first successful result
- Overhead of spawning and canceling tasks
- Comparison to parallel_vec()

**Expected results**:
- Race should return in time of fastest step + overhead
- Overhead should be <10ms regardless of step count

### Benchmark 4: Parallel Coordination Overhead
**Purpose**: Measure the pure overhead of parallel coordination (no I/O delay).

**Measures**:
- Spawn/join overhead per step
- Database write overhead
- JoinSet management overhead

**Expected results**:
- Overhead should be <5ms per step for 10 steps
- Should scale linearly with step count
- Most overhead should be from database writes

### Running Benchmarks

#### Prerequisites

1. Dedicated benchmark database:
   ```bash
   export DATABASE_URL="postgres://postgres:postgres@localhost:5432/kagzi_bench"
   ```
2. Ensure database is empty or reset before benchmarking:
   ```bash
   sqlx database drop --database-url $DATABASE_URL -y
   sqlx database create --database-url $DATABASE_URL
   sqlx migrate run --database-url $DATABASE_URL
   ```

#### Running All Benchmarks

```bash
cargo bench --package kagzi --bench parallel_performance
```

#### Running Specific Benchmarks

```bash
# Only parallel vs sequential
cargo bench --package kagzi --bench parallel_performance -- parallel_vs_sequential

# Only memoization
cargo bench --package kagzi --bench parallel_performance -- memoization

# Only race
cargo bench --package kagzi --bench parallel_performance -- race

# Only overhead
cargo bench --package kagzi --bench parallel_performance -- parallel_overhead
```

#### Viewing Benchmark Results

Results are saved to: `target/criterion/`

To view HTML reports:
```bash
open target/criterion/report/index.html
```

## Success Criteria

### Edge Case Tests
- ✅ All 7 edge case tests pass
- ✅ No race conditions detected
- ✅ Proper error handling for all failure modes
- ✅ Nested parallelism works correctly
- ✅ Retry integration functions properly

### Performance Requirements
- ✅ Parallel execution shows 2x+ speedup for 3+ parallel steps
- ✅ Memoization overhead <5ms per step
- ✅ Race() returns in time of fastest step + <10ms overhead
- ✅ Coordination overhead scales linearly

### Integration Requirements
- ✅ All tests pass in CI environment
- ✅ No memory leaks with large-scale tests
- ✅ Database connection pooling works correctly
- ✅ Worker gracefully handles parallel execution

## Known Limitations

### Current Implementation

1. **Tuple-based parallel()**: Currently only 2-tuple is implemented. Macros for 3-10 tuples are stubs.
2. **Error Strategy**: Only FailFast is fully implemented. CollectAll is defined but not fully tested.
3. **Nested Parallelism Depth**: No explicit limit on nesting depth; deep nesting may cause performance issues.

### Future Improvements

1. **Dynamic Concurrency Limiting**: Add semaphore-based limiting for very large parallel groups
2. **Priority-based Execution**: Allow prioritizing certain parallel steps
3. **Partial Retry**: Retry only failed steps in a parallel group
4. **Metrics Collection**: Track parallel execution metrics (speedup, cache hit rate, etc.)

## Troubleshooting

### Test Failures

#### Database Connection Issues
```
Error: Failed to connect to database
```
**Solution**: Ensure PostgreSQL is running and DATABASE_URL is correct.

#### Timeout Errors
```
Error: Test timed out after 10 seconds
```
**Solution**: Increase timeout or check if database is under heavy load.

#### Race Condition False Positives
```
Error: Expected 1 execution, got 2
```
**Solution**: This may indicate an actual race condition. Check database logs and ON CONFLICT behavior.

### Benchmark Issues

#### High Variance
**Symptom**: Benchmark results vary significantly between runs.
**Solution**:
- Ensure no other processes are competing for resources
- Increase sample size in benchmark config
- Use dedicated benchmark database

#### Slower Than Expected
**Symptom**: Parallel execution not showing expected speedup.
**Solution**:
- Check database connection pool size
- Verify no throttling or rate limiting
- Profile with `cargo flamegraph`

## Continuous Integration

### CI Pipeline

Add to `.github/workflows/test.yml`:

```yaml
- name: Run parallel execution tests
  run: |
    export DATABASE_URL="postgres://postgres:postgres@localhost:5432/kagzi_test"
    cargo test --package kagzi --test parallel_edge_cases_test -- --ignored

- name: Run benchmarks (quick)
  run: |
    export DATABASE_URL="postgres://postgres:postgres@localhost:5432/kagzi_bench"
    cargo bench --package kagzi --bench parallel_performance -- --quick
```

## Conclusion

The Week 5 test suite provides comprehensive coverage of parallel execution edge cases, integration scenarios, and performance characteristics. All tests passing indicates that the parallel execution feature is production-ready.

For questions or issues, refer to:
- `ROADMAP.md` for overall implementation plan
- `IMPLEMENTATION.md` for detailed feature specifications
- `README.md` for general usage
