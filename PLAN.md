# Plan: Concurrent Scratchpad Reservation and Upload (v3)

## 1. Goal

Improve the efficiency and robustness of the `anthill-cli put` command. Enable concurrent reservation and upload with concurrency limits. Introduce **immediate indexing of reserved scratchpads** (marking them appropriately) for fault tolerance and implement **retry mechanisms** for core operations (reservation, upload, commit, index updates). Provide three distinct progress bars: Reservation, Upload (Bytes), and Confirmed Scratchpads (Steps).

## 2. Current Process Analysis

Based on the `PutEvent` variants and the callback structure in `anthill-cli/src/callbacks/put.rs`, the current process appears largely sequential:

1.  **Check Need:** Determine if scratchpads are needed (`ReservingScratchpads`).
2.  **Confirmation:** If needed, prompt the user for confirmation (`ConfirmReservation`). This step blocks further progress until confirmed.
3.  **Reservation:** Reserve the required scratchpads one by one, updating progress (`ReservationProgress`).
4.  **Upload Start:** Once all reservations are complete (or if none were needed), signal the start of the upload (`StartingUpload`). The reservation progress bar is removed.
5.  **Upload:** Upload data, updating progress (`UploadProgress`). This might switch to step-based progress later (`ScratchpadUploadComplete`).
6.  **Completion:** Signal completion (`UploadFinished`, `StoreComplete`). The upload progress bar is removed.

The key bottleneck is step 3 blocking step 5.

## 3. Proposed Architecture (`anthill-lib`)

1.  **Concurrency Control:**
    *   Use `futures::stream::StreamExt::for_each_concurrent` or similar mechanisms (like `tokio::task::JoinSet` possibly combined with `tokio::sync::Semaphore` for fine-grained limits).
    *   Limit concurrent reservation operations to **20**.
    *   Limit concurrent upload *data transfer* operations to **20**.
    *   *(Assumption: Commit/confirmation operations are relatively quick and don't need a separate strict limit initially, might share the upload worker pool or run immediately after upload)*.

2.  **Master Index Integration & States:**
    *   The master index associated with the target key needs robust updates. Assume methods like `index.add_scratchpad(id, status)` and `index.update_scratchpad_status(id, new_status)` exist.
    *   **Required Index States (Examples):**
        *   `RESERVED_PENDING_UPLOAD`: Scratchpad successfully reserved via API *and* entry added to the index. Ready for data.
        *   `COMMITTED`: Scratchpad data uploaded, confirmed by the backend, and index entry updated (e.g., with final hash or metadata).
        *   *(Potentially others like `UPLOADING`, `COMMITTING` if finer granularity is desired, but let's start simpler).*
    *   **Reservation Flow:** Reserve via API -> **Update Index (Add entry with status `RESERVED_PENDING_UPLOAD`)** -> Push ID to `upload_ready_queue`.
    *   **Upload/Commit Flow:** Consume ID from queue -> Upload Data -> Commit/Confirm via API -> **Update Index (Update status to `COMMITTED`)**.

3.  **Retry Mechanisms:**
    *   Implement retry logic (e.g., using `tokio-retry` crate or a manual loop with exponential backoff, configurable max attempts like 3-5) for:
        *   Reservation API calls.
        *   Master index updates (both adding `RESERVED_PENDING_UPLOAD` and updating to `COMMITTED`).
        *   Upload data transfer operations.
        *   Commit/Confirmation API calls.
    *   A persistent failure (max retries exceeded) in *any* critical step for *any* scratchpad should halt the overall `put` operation and return a detailed error, allowing the user to potentially investigate or retry later. The index state should reflect the last successful step for the failed scratchpad.

4.  **Reservation & Upload Queue:**
    *   Use an MPSC channel (e.g., `tokio::sync::mpsc::channel`), named `upload_ready_queue`.
    *   It will hold identifiers (e.g., scratchpad IDs) of scratchpads that have been successfully reserved *and* indexed with `RESERVED_PENDING_UPLOAD`.

5.  **Modified `put` Flow:**
    *   **Initial Check:** Determine existing *usable* scratchpads (e.g., those marked `COMMITTED` but potentially part of a different logical file, or truly free ones) and calculate the number of *new* reservations needed. *(Handling partially failed uploads using `RESERVED_PENDING_UPLOAD` pads is part of the *resume* feature, not this initial implementation)*.
    *   **Confirmation:** Trigger `ConfirmReservation` if new reservations are needed. Block until user confirms.
    *   **Queue Initial Pads:** Push IDs of *existing usable* scratchpads determined in the initial check into the `upload_ready_queue`. *(This assumes the library can differentiate pads belonging to the current upload target vs other data)*.
    *   **Start Concurrent Tasks:**
        *   **Reservation Tasks (Limit 20):**
            1.  Attempt reservation via API (with retries).
            2.  On success, attempt to update the master index (with retries) to add the scratchpad with status `RESERVED_PENDING_UPLOAD`.
            3.  On successful indexing, push the scratchpad ID to `upload_ready_queue`.
            4.  Emit `ReservationProgress` only after *successful indexing*.
        *   **Upload Tasks (Limit 20):**
            1.  Consume scratchpad ID from `upload_ready_queue`.
            2.  Attempt data upload to the scratchpad (with retries). Update aggregate byte count for `UploadProgress`.
            3.  On successful upload, attempt commit/confirmation via API (with retries).
            4.  On successful commit, attempt final index update (with retries) to mark status as `COMMITTED`.
            5.  Emit `ScratchpadCommitComplete` only after *successful final indexing*.
    *   **Event Emission:**
        *   `ReservingScratchpads`: Emitted early.
        *   `ConfirmReservation`: Unchanged role.
        *   `ReservationProgress`: Emitted after successful reservation *and* initial indexing for a scratchpad. `current` tracks successfully indexed reservations, `total` is the number initially needed.
        *   `StartingUpload`: Emitted when the first scratchpad ID (either pre-existing or newly reserved) is pushed to the `upload_ready_queue`. Reports `total_bytes`.
        *   `UploadProgress`: Aggregated byte progress from successful data transfer attempts across all concurrent upload tasks.
        *   `ScratchpadCommitComplete`: Emitted after successful commit *and* final indexing for a scratchpad. `index` represents the count of committed pads, `total` is the total number of pads involved in the upload.
        *   `StoreComplete`: Emitted only after *all* required reservation+indexing tasks and *all* required upload+commit+indexing tasks have completed successfully (within retry limits).
    *   **Completion & Error Handling:** The `put` function returns `Ok` only if all steps succeed within retry limits. Any persistent failure halts the process and returns an `Err`, leaving the index in a state reflecting the last successful operations (enabling future resume).

## 4. Proposed Changes (`anthill-cli/src/callbacks/put.rs`)

(No significant changes from v2 needed here; the CLI callback layer reacts to the events as defined above. The complexity lies within the library emitting these events correctly based on the new indexing and retry logic).

1.  **State Management:** Maintain `Arc<Mutex<Option<StyledProgressBar>>>` for `reservation_pb_opt`, `upload_bytes_pb_opt`, `confirmed_pads_pb_opt`.
2.  **Event Handling:** Structure remains the same as v2, interpreting events based on their refined meanings (post-indexing, post-commit-indexing).

## 5. Implementation Steps (Incremental)

**(Current Status: Starting Step 11)**

1.  **DONE: (Lib) Define/Confirm master index states (`RESERVED_PENDING_UPLOAD`, `COMMITTED`). Ensure index update functions are available.** (Commit: `337fb3c`)
    *   [x] Located index implementation (`anthill-lib/src/anthill/data_structures.rs`).
    *   [x] Defined `PendingScratchpadStatus` enum with `Reserved`, `Committed`.
    *   [x] Defined `PendingScratchpadInfo` struct to hold address, key, and status.
    *   [x] Added `pending_uploads: HashMap<String, Vec<PendingScratchpadInfo>>` to `MasterIndexStorage` with `#[serde(default)]` and `secret_key_serde` helper.
    *   [x] Fixed initialization of `MasterIndexStorage` in `storage.rs`.
    *   [x] Ran `cargo check --workspace` - Passed.
    *   [x] Committed changes.
2.  **DONE: (Lib) Choose and integrate a retry library (like `tokio-retry`) or implement a robust retry helper.** (Commit: `54f954e`)
    *   [x] Added `tokio-retry` dependency to `anthill-lib/Cargo.toml`.
    *   [x] Created a helper module (`anthill-lib/src/utils/retry.rs`) with `standard_retry_strategy` and `retry_operation` function.
    *   [x] Fixed closure lifetime issues by using `Fn` instead of `FnMut`.
    *   [x] Ran `cargo check --workspace` - Passed.
    *   [x] Committed changes.
3.  **DONE: (Lib) Implement concurrent reservation tasks.** (Commit: `8f1132e`)
    *   [x] Refactored `anthill::store_logic::store_item` to handle concurrency.
    *   [x] Used `futures::stream::iter` and `buffer_unordered(RESERVATION_CONCURRENCY)` + `try_collect`.
    *   [x] Inside the concurrent task:
        *   Called `storage.create_scratchpad_internal_raw` (simplified reservation).
        *   Wrapped reservation API call and index update in `utils::retry::retry_operation`.
        *   Updated `MasterIndexStorage.pending_uploads` with `Reserved` status after successful reservation+retry (locking MIS).
        *   Pushed tuple `(usize, ScratchpadAddress, SecretKey)` to `upload_ready_tx`.
        *   Emitted `ReservationProgress` event after successful indexing.
    *   [x] Defined `RESERVATION_CONCURRENCY` constant (20).
    *   [x] Resolved various linter/compiler errors (Mutex, ConfirmResult, Error paths, `!`).
    *   [x] Ran `cargo check --workspace` - Passed.
    *   [x] Committed changes.
4.  **DONE: (Lib) Introduce `upload_ready_queue`.** (Commit: `6d398a7`)
    *   [x] Defined MPSC channel `(usize, ScratchpadAddress, SecretKey)` in `store_item`.
    *   [x] Ensured `Sender` is cloned for reservation stream and original sender is dropped correctly.
    *   [x] Added placeholder logic for handling initial existing free pads.
    *   [x] Added placeholder upload task consuming from receiver.
    *   [x] Used `tokio::try_join!` to run reservation and placeholder upload concurrently.
    *   [x] Fixed compiler errors (dereference, display format).
    *   [x] Ran `cargo check` in workspace - Passed.
    *   [x] Committed changes.
5.  **DONE: (Lib) Implement concurrent upload tasks.** (Commit: `8f2b05e`)
    *   [x] Replaced placeholder upload loop with concurrent logic using `tokio::spawn`, `Semaphore`, `JoinSet`.
    *   [x] Inside the concurrent task:
        *   Received `(usize, ScratchpadAddress, SecretKey)` from `upload_ready_rx`.
        *   Determined data chunk based on index.
        *   Called `storage.update_scratchpad_internal` (upload) with retry.
        *   (Placeholder for commit API call).
        *   Updated `MasterIndexStorage.pending_uploads` status to `Committed` (with retry).
        *   Emitted placeholder `ScratchpadUploadComplete` event after commit+indexing.
    *   [x] Handled task results and potential errors using `JoinSet::join_next`.
    *   [x] Fixed callback borrowing (`E0499`) using shared `Arc<Mutex<...>>`.
    *   [x] Fixed placeholder value types (`E0308`).
    *   [x] Added `JoinSet` type annotation.
    *   [x] Ran `cargo check` in workspace - Passed.
    *   [x] Committed changes.
6.  **DONE: (Lib) Implement aggregated `UploadProgress` reporting.** (Commit: `5609afd`)
    *   [x] Added shared state `total_bytes_uploaded = Arc<Mutex<u64>>`.
    *   [x] Inside the upload task (after successful `update_scratchpad_internal` retry block):
        *   Locked `total_bytes_uploaded`.
        *   Added size of `data_chunk` to it.
        *   Read the current total.
        *   Emitted `PutEvent::UploadProgress { bytes_written: current_total, total_bytes: data_size }` using shared callback Arc.
    *   [x] Fixed scope errors in retry closure.
    *   [x] Fixed data chunk calculation placeholder.
    *   [x] Ran `cargo check` in workspace - Passed.
    *   [x] Committed changes.
7.  **DONE: (Lib) Adjust `StartingUpload`, `StoreComplete` logic. Handle queueing initial pads. Implement robust error handling.** (Commit: `f8bfd3f`)
    *   [x] Emitted `StartingUpload` event just before the `try_join!` block, ensuring `total_bytes` (overall `data_size`) is included.
    *   [x] Moved final `StoreComplete` emission and final index update/cleanup (removing `pending_uploads` entry) *inside* the `Ok` branch of the `try_join!` result (using placeholders).
    *   [x] Added placeholder comments for querying/queueing existing usable pads.
    *   [x] Added placeholder comments for cleanup in `try_join!` `Err` branch.
    *   [x] Defined `ScratchpadCommitComplete` event in `events.rs`.
    *   [x] Used `ScratchpadCommitComplete` event in upload task.
    *   [x] Added `todo!()` arm for new event in CLI callback.
    *   [x] Ran `cargo check` in workspace - Passed.
    *   [x] Committed changes.
8.  **DONE: (CLI) Add state for `confirmed_pads_pb_opt`.** (Commit: `5bacb56`)
    *   [x] Added `confirmed_pads_pb_opt: Arc<Mutex<Option<StyledProgressBar>>>` to `create_put_callback` state tuple.
    *   [x] Updated return type and tuple destructuring.
    *   [x] Cloned and passed the new Arc into the callback closure.
    *   [x] Ran `cargo check --workspace` - Passed.
    *   [x] Committed changes.
9.  **DONE: (CLI) Implement/Adjust event handlers for the three progress bars.** (Commit: `e346417`)
    *   [x] `ConfirmReservation`: Initialize `reservation_pb` (step style).
    *   [x] `ReservationProgress`: Update `reservation_pb` position. Finish it when `current == total`.
    *   [x] `StartingUpload`: Initialize `upload_bytes_pb` (byte style).
    *   [x] `UploadProgress`: Update `upload_bytes_pb` position.
    *   [x] `ScratchpadCommitComplete`: Initialize `confirmed_pads_pb` (step style) on first call, update position.
    *   [x] Ran `cargo check --workspace` - Passed.
    *   [x] Committed changes.
10. **DONE: (CLI) Implement bar cleanup logic.** (Commit: `8ac6084`)
    *   [x] In `StoreComplete` handler: Finish and clear `upload_bytes_pb` and `confirmed_pads_pb`.
    *   [x] In `handle_put` (command handler) error/cancellation branches: Ensure all three bars (`reservation_pb`, `upload_pb`, `confirmed_pads_pb`) are cleaned up (`finish_and_clear` or `abandon_with_message`).
    *   [x] Fixed tuple destructuring and `.take()` errors.
    *   [x] Ran `cargo check --workspace` - Passed.
    *   [x] Committed changes.
11. **(Testing) Thoroughly test success paths, retry scenarios, and failure scenarios.**

## 6. Progress Bar Management

(Same as v2)

## 7. Resume Capability Foundation

(Same as v2)