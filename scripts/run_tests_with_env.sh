#!/bin/bash
set -e # Exit immediately if a command exits with a non-zero status.

# Determine PROJECT_ROOT based on the script's location
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
# Project root is one level above the script directory
PROJECT_ROOT=$( cd -- "$SCRIPT_DIR/.." &> /dev/null && pwd )

# Path to the management script (relative to the project root)
MANAGE_SCRIPT="${PROJECT_ROOT}/scripts/manage_local_testnet.sh"

# Check for optional test path argument
TEST_PATH_FILTER="${1:-}" # Default to empty if no argument

echo "INFO: Using Project Root: $PROJECT_ROOT"
echo "INFO: Using Management Script: $MANAGE_SCRIPT"

# --- Ensure Management Script Exists and is Executable ---
if [ ! -x "$MANAGE_SCRIPT" ]; then
    echo "ERROR: Management script not found or not executable: $MANAGE_SCRIPT"
    # Try to make it executable if it exists but isn't executable
    if [ -f "$MANAGE_SCRIPT" ]; then
        echo "Attempting to make it executable..."
        chmod +x "$MANAGE_SCRIPT"
        if [ ! -x "$MANAGE_SCRIPT" ]; then
            echo "ERROR: Failed to make management script executable."
            exit 1
        fi
    else
        exit 1
    fi
fi


# --- Cleanup Function ---
cleanup() {
  echo
  echo "--- Cleaning up ---"
  # Call the management script to stop the network
  echo "Stopping local testnet using manage script..."
  if "$MANAGE_SCRIPT" stop; then
      echo "Local testnet stopped successfully via manage script."
  else
      echo "WARN: Manage script failed to stop the network (exit code $?). Might already be stopped or script has issues."
      # Add manual fallback kill commands if needed, but ideally manage script handles it
  fi
  echo "Cleanup finished."
}

# Trap signals to ensure cleanup runs even on error or interrupt
trap cleanup EXIT SIGINT SIGTERM

# --- Prepare Dependencies (using manage script) ---
echo "--- Preparing Dependencies (using manage script) ---"
if ! "$MANAGE_SCRIPT" setup; then
    echo "ERROR: Failed to setup dependencies using management script."
    exit 1
fi

# --- Start Local Testnet (using manage script) ---
echo "--- Starting Local Testnet (using manage script) ---"
if ! "$MANAGE_SCRIPT" start; then
    echo "ERROR: Failed to start local testnet using management script."
    # Cleanup will run via trap
    exit 1
fi
echo "Local testnet started successfully."

# --- Run Tests ---
echo "--- Running Tests ---"
# Set XDG_DATA_HOME for the test environment to match the manage script
export XDG_DATA_HOME="${PROJECT_ROOT}/test_network_data"
echo "INFO: Exporting XDG_DATA_HOME for tests: $XDG_DATA_HOME"

# Verify cargo exists in path before running
if ! command -v cargo &> /dev/null; then
    echo "ERROR: cargo command not found in PATH"
    exit 1
fi

echo "Running cargo test (serially)..."
# Run tests from the project root
cd "$PROJECT_ROOT" || exit 1

# Initialize overall exit code
OVERALL_TEST_EXIT_CODE=0

# Test the cli (or other workspace tests if desired)
echo "--- Testing CLI Crate ---"
if cargo test --package mutant-cli -- --nocapture --test-threads=1; then
  echo "CLI tests Passed"
else
  CLI_EXIT_CODE=$?
  echo "CLI tests Failed (Exit Code: $CLI_EXIT_CODE)"
  OVERALL_TEST_EXIT_CODE=$CLI_EXIT_CODE # Record failure
fi

# Test the mutant library
echo "--- Testing Lib Crate (Filter: '${TEST_PATH_FILTER:-<all>}') ---"
LIB_TEST_CMD="cargo test --package mutant-lib -- --nocapture --test-threads=1"
if [ -n "$TEST_PATH_FILTER" ]; then
  LIB_TEST_CMD="$LIB_TEST_CMD $TEST_PATH_FILTER"
fi

echo "Executing: $LIB_TEST_CMD"
if eval "$LIB_TEST_CMD"; then # Use eval to handle potential spaces in filter correctly
  echo "MutAnt lib tests passed!"
else
  LIB_EXIT_CODE=$?
  echo "MutAnt lib tests failed (Exit Code: $LIB_EXIT_CODE)."
  if [ $OVERALL_TEST_EXIT_CODE -eq 0 ]; then # Only update if not already failed
      OVERALL_TEST_EXIT_CODE=$LIB_EXIT_CODE
  fi
fi

# --- Cleanup --- is handled automatically by the trap ---

# Exit with the overall test result code
if [ $OVERALL_TEST_EXIT_CODE -eq 0 ]; then
    echo "--- All Tests Passed --- "
else
    echo "--- Some Tests Failed (Overall Exit Code: $OVERALL_TEST_EXIT_CODE) --- "
fi
exit $OVERALL_TEST_EXIT_CODE 