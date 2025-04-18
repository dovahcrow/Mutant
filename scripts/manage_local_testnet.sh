#!/bin/bash
set -e # Exit immediately if a command exits with a non-zero status.

# Determine PROJECT_ROOT based on the script's location
SCRIPT_DIR=$( cd -- "$( dirname -- "${BASH_SOURCE[0]}" )" &> /dev/null && pwd )
# Project root is one level above the script directory
PROJECT_ROOT=$( cd -- "$SCRIPT_DIR/.." &> /dev/null && pwd )

AUTONOMI_DIR="${PROJECT_ROOT}/.test_deps/autonomi"
AUTONOMI_REPO="https://github.com/maidsafe/autonomi.git"
EVM_PID_FILE="/tmp/anthill_evm_testnet.pid"
EVM_LOG_FILE="/tmp/anthill_evm_testnet.log"

# Define data path for test network and override XDG_DATA_HOME
TEST_NETWORK_DATA_PATH="${PROJECT_ROOT}/test_network_data"
export XDG_DATA_HOME="$TEST_NETWORK_DATA_PATH"

# Ensure the target data directory exists early
mkdir -p "$XDG_DATA_HOME"

echo "INFO: Using PROJECT_ROOT: $PROJECT_ROOT"
echo "INFO: Using AUTONOMI_DIR: $AUTONOMI_DIR"
echo "INFO: Using XDG_DATA_HOME: $XDG_DATA_HOME"
echo "INFO: Using EVM_PID_FILE: $EVM_PID_FILE"
echo "INFO: Using EVM_LOG_FILE: $EVM_LOG_FILE"


# --- Dependency Setup ---
setup_deps() {
    echo "--- Checking/Setting up Autonomi Dependency ---"
    if [ ! -d "$AUTONOMI_DIR/.git" ]; then
        echo "Cloning Autonomi repository into $AUTONOMI_DIR..."
        mkdir -p "$(dirname "$AUTONOMI_DIR")"
        git clone --depth 1 "$AUTONOMI_REPO" "$AUTONOMI_DIR"
    else
        echo "Autonomi repository found in $AUTONOMI_DIR."
        # Optional: Update existing repo
        # echo "Updating Autonomi repository..."
        # (cd "$AUTONOMI_DIR" && git pull)
    fi
    # Consider building antctl here if needed universally, or keep it in start_antctl
}

# --- EVM Management ---
start_evm() {
    echo "--- Starting EVM Testnet ---"
    if is_evm_running; then
        echo "EVM testnet appears to be running already."
        return 0
    fi

    local evm_binary="$AUTONOMI_DIR/target/debug/evm-testnet"
    if [ ! -x "$evm_binary" ]; then
       echo "Building evm-testnet in $AUTONOMI_DIR..."
       if ! (cd "$AUTONOMI_DIR" && cargo build --bin evm-testnet); then
           echo "ERROR: Failed to build evm-testnet in $AUTONOMI_DIR" >&2
           # Optionally dump logs or target dir contents here
           ls -l "$AUTONOMI_DIR/target/debug"
           return 1 # Indicate failure
       fi
       echo "evm-testnet build completed."
       # Verify it exists now
       if [ ! -x "$evm_binary" ]; then
            echo "ERROR: evm-testnet binary not found at $evm_binary even after successful build command!" >&2
            ls -l "$AUTONOMI_DIR/target/debug"
            return 1
       fi
    fi

    local pgrep_pattern="$evm_binary" # Use full path for pgrep pattern

    echo "Starting EVM testnet directly in the background (Logging to $EVM_LOG_FILE)..."
    # Launch directly in the background without setsid
    (cd "$AUTONOMI_DIR" && "$evm_binary" &> "$EVM_LOG_FILE" &)
    local bg_launcher_pid=$! # PID of the subshell that launched the process
    echo "Launched background command (Launcher PID: $bg_launcher_pid) to start evm-testnet."

    # Give it time to start
    sleep 5
    # Give it more time to start
    echo "Waited 5 seconds for EVM testnet to initialize..."

    # Find the actual EVM_PID using pgrep with the full path
    EVM_PID=$(pgrep -f "$pgrep_pattern")

    # Validate the PID found by pgrep
    if [ -z "$EVM_PID" ]; then
        echo "ERROR: Could not find running EVM testnet process using pgrep pattern '$pgrep_pattern'."
        echo "--- Dumping EVM Log ($EVM_LOG_FILE) ---"
        \cat "$EVM_LOG_FILE" || echo "Log file empty or does not exist."
        echo "--- Checking process table with ps aux | grep --- "
        ps aux | \grep '[e]vm-testnet' || echo "No matching process found with ps aux."
        echo "--- Checking process table with pgrep -af --- "
        pgrep -af "$pgrep_pattern" || echo "No matching process found with pgrep -af."
        echo "--- End Diagnostics ---"
        return 1
    elif [ $(echo "$EVM_PID" | wc -l) -ne 1 ]; then
        echo "ERROR: Found multiple EVM testnet processes matching pattern '$pgrep_pattern'. PIDs: $EVM_PID. Please stop them manually." >&2
        # Attempt to kill them all to clean up
        pkill -KILL -f "$pgrep_pattern"
        return 1
    elif ! [[ "$EVM_PID" =~ ^[0-9]+$ ]]; then
         echo "ERROR: pgrep found non-numeric PID: '$EVM_PID'."
         return 1
    fi

    # Double check the found process is running
    if ! \ps -p $EVM_PID > /dev/null; then
       echo "ERROR: Found PID $EVM_PID with pgrep, but process is not running. Check logs: $EVM_LOG_FILE"
       # Clean up potentially incorrect PID file if it was somehow written
       if [ -f "$EVM_PID_FILE" ] && [ "$(cat $EVM_PID_FILE)" == "$EVM_PID" ]; then rm -f "$EVM_PID_FILE"; fi
       return 1
    fi

    echo $EVM_PID > "$EVM_PID_FILE" # Still useful for status/quick checks
    echo "EVM testnet started (PID: $EVM_PID)."
}

stop_evm() {
    echo "--- Stopping EVM Testnet ---"
    local evm_binary="$AUTONOMI_DIR/target/debug/evm-testnet"
    local pgrep_pattern="$evm_binary"
    local found_pids=$(pgrep -f "$pgrep_pattern")

    if [ -n "$found_pids" ]; then
        echo "Found running EVM testnet process(es) matching pattern '$pgrep_pattern' with PID(s): $found_pids"
        echo "Attempting to stop using pkill..."
        # Use pkill to send TERM, then check and send KILL if needed
        if pkill -TERM -f "$pgrep_pattern"; then
            echo "Sent TERM signal via pkill."
            sleep 1
            # Check if any are still running
            local remaining_pids=$(pgrep -f "$pgrep_pattern")
            if [ -n "$remaining_pids" ]; then
                 echo "Processes still running (PIDs: $remaining_pids) after TERM, sending KILL signal via pkill..."
                 pkill -KILL -f "$pgrep_pattern" || echo "pkill -KILL failed, processes might have stopped anyway."
            else
                echo "Processes stopped successfully after TERM signal."
            fi
        else
             echo "pkill -TERM failed. Processes matching pattern might have already stopped."
        fi
    else
        echo "No running EVM testnet process found via pgrep pattern '$pgrep_pattern'."
    fi

    # Clean up PID file regardless
    if [ -f "$EVM_PID_FILE" ]; then
        echo "Removing PID file ($EVM_PID_FILE)."
        rm -f "$EVM_PID_FILE"
    else
        echo "EVM PID file not found ($EVM_PID_FILE), no file to remove."
    fi
}

is_evm_running() {
    local evm_binary="$AUTONOMI_DIR/target/debug/evm-testnet"
    local pgrep_pattern="$evm_binary"
    local found_pid=$(pgrep -f "$pgrep_pattern" | head -n 1) # Get the first PID if multiple exist

    if [ -n "$found_pid" ] && [[ "$found_pid" =~ ^[0-9]+$ ]]; then
        # If running, ensure PID file exists and is correct
        if [ -f "$EVM_PID_FILE" ]; then
            local file_pid=$(cat "$EVM_PID_FILE")
            if [ "$found_pid" != "$file_pid" ]; then
                 echo "WARN: Running EVM PID ($found_pid) differs from PID file ($file_pid). Updating PID file."
                 echo "$found_pid" > "$EVM_PID_FILE"
            fi
        else
            echo "INFO: EVM running (PID $found_pid) but no PID file found. Creating PID file."
            echo "$found_pid" > "$EVM_PID_FILE"
        fi
        return 0 # Running
    else
        # Not running according to pgrep, clean up PID file if it exists
        if [ -f "$EVM_PID_FILE" ]; then
            echo "INFO: EVM not running (pgrep check failed), but PID file exists. Removing stale PID file ($EVM_PID_FILE)."
            rm -f "$EVM_PID_FILE"
        fi
        return 1 # Not running
    fi
}

# --- Antctl Management ---
start_antctl() {
    echo "--- Starting Antctl Local Network ---"
    local antctl_binary="$AUTONOMI_DIR/target/debug/antctl"

    # Ensure antctl is built
    echo "Building antctl in $AUTONOMI_DIR..."
    if ! (cd "$AUTONOMI_DIR" && cargo build --features=open-metrics --bin antctl); then
        echo "ERROR: Failed to build antctl in $AUTONOMI_DIR" >&2
        ls -l "$AUTONOMI_DIR/target/debug"
        return 1 # Indicate failure
    fi
    echo "antctl build completed."
    # Verify it exists now
    if [ ! -x "$antctl_binary" ]; then
        echo "ERROR: antctl binary not found at $antctl_binary even after successful build command!" >&2
        ls -l "$AUTONOMI_DIR/target/debug"
        return 1
    fi

    echo "Stopping any previous antctl local network..."
    # Allow kill to fail if network isn't running. Should respect XDG_DATA_HOME.
    (cd "$AUTONOMI_DIR" && ./target/debug/antctl local kill) || echo "No previous antctl network to kill, or kill command failed."

    echo "Starting new antctl local network (this may take a while)..."
    # Should use the overridden XDG_DATA_HOME automatically
    if ! (cd "$AUTONOMI_DIR" && ./target/debug/antctl local run --build --clean --enable-metrics-server --rewards-address 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 evm-local); then
        echo "ERROR: Failed to start antctl local network."
        return 1
    fi
    echo "antctl local network started."
    echo "Waiting for network to stabilize (1s)..."
    sleep 1
}

stop_antctl() {
    echo "--- Stopping Antctl Local Network ---"
    echo "Using XDG_DATA_HOME: $XDG_DATA_HOME"
    local antctl_binary="$AUTONOMI_DIR/target/debug/antctl"

    # Check if antctl binary exists first
    if [ ! -x "$antctl_binary" ]; then
        echo "antctl binary not found at '$antctl_binary', cannot run kill command. Assuming network is stopped."
        # If the binary doesn't exist, we can't stop anything, but the network is effectively stopped.
        return 0
    fi

    # Run kill command from autonomi directory. It should respect XDG_DATA_HOME.
    if (cd "$AUTONOMI_DIR" && "$antctl_binary" local kill); then
        echo "antctl local network stopped successfully."
    else
        # The kill command failed, but the binary exists.
        # This usually means the network was already stopped.
        echo "antctl local kill command failed (exit code $?). Network might already be stopped."
        # Don't treat failure to stop as a script error if it might be stopped already
        return 0
    fi
}

is_antctl_running() {
    echo "--- Checking Antctl Status ---"
    # This is tricky. 'antctl local status' might not exist or work reliably.
    # A simple check could be looking for 'antnode' processes associated with the data dir.
    # Use pgrep -f -- --local which seems more reliable.
    if pgrep -f -- --local > /dev/null; then
        echo "Found processes matching '--local' via pgrep, assuming network is running."
        return 0
    else
        echo "Did not find processes matching '--local' via pgrep, assuming network is stopped."
        return 1
    fi
}

# --- Cleanup Data ---
cleanup_data() {
    echo "--- Cleaning up Network Data ---"
    if [ -d "$TEST_NETWORK_DATA_PATH" ]; then
        echo "Removing test network data directory: $TEST_NETWORK_DATA_PATH"
        rm -rf "$TEST_NETWORK_DATA_PATH"
        # Recreate the base dir
        mkdir -p "$XDG_DATA_HOME"

    else
        echo "Test network data directory not found: $TEST_NETWORK_DATA_PATH"
    fi
    # Also remove EVM logs/pid
    echo "Removing EVM log and PID files..."
    rm -f "$EVM_LOG_FILE" "$EVM_PID_FILE"
}


# --- Main Command Logic ---
COMMAND=$1
shift # Remove command from argument list

case $COMMAND in
    start)
        echo "Executing: start"
        setup_deps
        stop_antctl # Stop antctl first
        stop_evm    # Then stop EVM
        start_evm
        # Check if EVM started successfully before starting antctl
        if ! is_evm_running; then
            echo "ERROR: EVM failed to start, aborting antctl start."
            exit 1
        fi
        start_antctl
        echo "--- Local Testnet Started ---"
        ;;
    stop)
        echo "Executing: stop"
        # Order matters: stop antctl first as it might depend on EVM
        stop_antctl
        stop_evm
        echo "--- Local Testnet Stopped ---"
        ;;
    restart)
        echo "Executing: restart"
        $0 stop
        $0 start
        ;;
    status)
        echo "Executing: status"
        echo "--- EVM Status ---"
        if is_evm_running; then
            echo "EVM Testnet: Running (PID $(cat $EVM_PID_FILE))"
            echo "Log file: $EVM_LOG_FILE"
        else
            echo "EVM Testnet: Stopped"
        fi
        echo "--- Antctl Status ---"
        if is_antctl_running; then
             echo "Antctl Local Network: Likely Running (based on process check)"
        else
             echo "Antctl Local Network: Likely Stopped (based on process check)"
        fi
        ;;
    logs)
        echo "Executing: logs (EVM)"
        if [ -f "$EVM_LOG_FILE" ]; then
            echo "Tailing EVM logs ($EVM_LOG_FILE)..."
            tail -f "$EVM_LOG_FILE"
        else
            echo "EVM log file not found: $EVM_LOG_FILE"
            # Check if EVM is running without a log file (shouldn't happen with current start logic)
            if is_evm_running; then
                echo "WARN: EVM is running but log file is missing!"
            fi
        fi
        ;;
    setup)
        echo "Executing: setup (Cloning/Updating Autonomi)"
        setup_deps
        echo "--- Dependency Setup Complete ---"
        ;;
    clean)
        echo "Executing: clean (Stopping network and removing data)"
        $0 stop # Ensure network is stopped before removing data
        cleanup_data
        echo "--- Network Data Cleaned ---"
        ;;
    *)
        echo "Usage: $0 {start|stop|restart|status|logs|setup|clean}"
        exit 1
        ;;
esac

exit 0 