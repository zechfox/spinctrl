#!/bin/bash
# SpinCtrl bash service test runner.
# Defines mock hardware, sources helpers + the service script (main() NOT
# run due to the BASH_SOURCE guard), silences noisy functions, then runs
# all test_ functions and prints a summary. Exits 0 on all-pass, 1 on any fail.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Mock hardware functions BEFORE sourcing the service script so bash resolves
# these instead of real ectool/cpupower (no hardware needed, hermetic).
ectool() {
    case "$1" in
        chargecontrol) return 0 ;;
        thermalget) echo "sensor 0: 343K 333K 353K 323K 348K"; return 0 ;;
        thermalset) return 0 ;;
        *) return 0 ;;
    esac
}
cpupower() {
    case "$1" in
        frequency-info) echo "current policy: performance"; return 0 ;;
        frequency-set) return 0 ;;
        *) return 0 ;;
    esac
}

# Source test helpers
source "$SCRIPT_DIR/helpers.sh"

# Source the service script (imports functions; main() NOT run due to the
# BASH_SOURCE guard)
# shellcheck source=/dev/null
source "$REPO_DIR/spinctrl-service/spinctrl-service.sh"

# Override noisy functions so tests are quiet and don't touch /var/lib or journal
log() { :; }
write_event() { :; }
rotate_events_log() { :; }

# Source test cases
source "$SCRIPT_DIR/test_service.sh"

# Discover and run all test_ functions
echo "Running SpinCtrl service tests..."
for test_fn in $(declare -F | awk '/declare -f test_/ {print $3}'); do
    run_test "$test_fn"
done

print_summary
