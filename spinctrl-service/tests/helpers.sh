#!/bin/bash
# SpinCtrl bash test helpers: assert functions, mock hardware, reporter.
# Sourced by run.sh before the test cases.

TEST_COUNT=0
TEST_PASS=0
TEST_FAIL=0
_TEST_FAILED=0

assert_eq() {
    local actual="$1" expected="$2" msg="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "  PASS  $msg"
    else
        echo "  FAIL  $msg (expected '$expected', got '$actual')"
        _TEST_FAILED=1
    fi
}

assert_contains() {
    local haystack="$1" needle="$2" msg="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        echo "  PASS  $msg"
    else
        echo "  FAIL  $msg ('$haystack' does not contain '$needle')"
        _TEST_FAILED=1
    fi
}

assert_match() {
    local regex="$1" str="$2" msg="$3"
    if [[ "$str" =~ $regex ]]; then
        echo "  PASS  $msg"
    else
        echo "  FAIL  $msg ('$str' does not match /$regex/)"
        _TEST_FAILED=1
    fi
}

run_test() {
    local test_fn="$1"
    _TEST_FAILED=0
    echo ""
    echo "--- $test_fn ---"
    "$test_fn"
    TEST_COUNT=$((TEST_COUNT + 1))
    if [[ $_TEST_FAILED -eq 0 ]]; then
        TEST_PASS=$((TEST_PASS + 1))
        echo "  RESULT: PASS"
    else
        TEST_FAIL=$((TEST_FAIL + 1))
        echo "  RESULT: FAIL"
    fi
}

print_summary() {
    echo ""
    echo "================================"
    echo "Tests: $TEST_COUNT  Pass: $TEST_PASS  Fail: $TEST_FAIL"
    echo "================================"
    [[ $TEST_FAIL -eq 0 ]]
}

# Replicates the thermal read-back verification logic from configure_thermal.
# Args: <readback-string> <expected-kelvin-values...>
# Returns 0 if all expected values appear in the readback, 1 otherwise.
check_thermal_readback() {
    local readback="$1"; shift
    local nums all_present v
    nums=$(grep -oE '[0-9]+' <<<"$readback" 2>/dev/null)
    all_present=1
    for v in "$@"; do
        grep -qx "$v" <<<"$nums" 2>/dev/null || all_present=0
    done
    [[ $all_present -eq 1 ]]
}
