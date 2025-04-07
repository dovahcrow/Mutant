# Refactoring Plan: `store_item` to use `MemoryAllocator`

**Goal:** Refactor the `anthill-lib/src/anthill/store_logic.rs::store_item` function to delegate scratchpad management, data allocation, and writing entirely to the `MemoryAllocator`. This will ensure the allocator's internal state is accurate, fixing the issue where the confirmation prompt shows incorrect storage/pad info, **while preserving the concurrent reservation/upload behavior from PLAN.md (V3).**

**Current Implementation Issues:**

*   `store_item` manually calculates needed scratchpads.
*   `store_item` directly reserves scratchpads using `storage.create_scratchpad_internal_raw`.
*   `store_item` implements its own concurrent logic for writing data chunks to reserved pads.
*   `store_item` updates the `MasterIndex` with `DataLocation::Chunked` (or similar, depending on size/logic not yet implemented fully).
*   The `MemoryAllocator` is completely bypassed, causing its state (`MemoryMap`) to become stale regarding total pads and free space.

**Proposed `MemoryAllocator` Methods to Use:**

*   `MemoryAllocator::allocate(size, callback)`: Allocates space for the data. Handles finding free blocks or expanding storage (reserving new pads) if necessary. Returns a `DataId`. Crucially, it should update its internal `MemoryMap` state. **Must implement concurrent reservation internally if expansion occurs.**
*   `MemoryAllocator::write(data_id, data_bytes, callback)`: Writes the provided data to the location(s) associated with the `DataId`. Handles chunking/splitting across pads internally if the allocation spanned multiple pads. Manages upload progress reporting via the callback. **Must implement concurrent writing/committing internally.**
*   `MemoryAllocator::get_storage_and_pad_summary()`: (Already added) Used to get the *current* state for the confirmation prompt.

**Refactoring Steps:**

1.  **Analyze `MemoryAllocator::allocate` & `MemoryMap::expand`:**
    *   [ ] Verify that `allocate`, when triggering `MemoryMap::expand`, correctly updates the `MemoryMap` state (total pads, free blocks).
    *   [ ] **Concurrency Requirement:** Ensure `MemoryMap::expand` (called by `allocate`) performs scratchpad reservations concurrently (e.g., using `buffer_unordered` or similar, with `RESERVATION_CONCURRENCY` limit) similar to the V3 `reservation_future`.
    *   [ ] **Event Requirement:** Ensure `expand` emits `PutEvent::ReservationProgress` after each successful reservation + *internal state update* (e.g., map update, **not** master index update, as the allocator manages its own map persistence).
    *   [ ] **Confirmation Handling:** `expand` should *not* handle `ConfirmReservation`. Confirmation must happen in `store_item` *before* calling `allocate`.

2.  **Modify `store_item`:** (`anthill-lib/src/anthill/store_logic.rs`)
    *   [ ] **Pre-checks:** Keep the initial checks for key existence (`mis_guard.index.contains_key`) and pending uploads (`mis_guard.pending_uploads.contains_key`).
    *   [ ] **Remove Manual Pad Calculation:** Delete the code calculating `pads_needed` and `new_reservations_needed`.
    *   [ ] **Confirmation Logic (Revised):**
        *   Call `es.allocator.get_storage_and_pad_summary().await` *before* attempting allocation to get the *current* state.
        *   Call `es.allocator.check_allocation_possible(data_size).await` (New method needed - Step 3). 
        *   If expansion *is* needed (returns `Ok(Some(needed_pads))`), invoke the `PutEvent::ConfirmReservation` callback using the summary info obtained earlier and `needed: needed_pads`. Propagate cancellation errors (`Error::OperationCancelled`).
        *   If no expansion is needed (`Ok(None)`) or if the user confirmed, proceed to allocation.
    *   [ ] **Call `allocate`:** Replace the manual reservation loop (`reservation_future`) with `let data_id = es.allocator.allocate(data_size, &mut callback).await?;`. The allocator now handles concurrent reservation internally if needed.
    *   [ ] **Remove Manual Upload Logic:** Delete the entire concurrent upload section (`upload_future`, `upload_ready_tx/rx`, `mpsc`, chunking logic, manual index updates for `Committed` status, etc.).
    *   [ ] **Call `write`:** Add the call: `es.allocator.write(data_id, data_bytes, &mut callback).await?;`. The allocator handles concurrent writing and committing internally.
    *   [ ] **Update Master Index:** In the final success path:
        *   Lock the `MasterIndexStorage` (`mis_guard`).
        *   Store `DataLocation::Allocated(data_id)` in `mis_guard.index` for the key.
        *   Remove the entry from `mis_guard.pending_uploads` if it exists (cleanup from V3 logic, may no longer be strictly needed if allocator doesn't use it, but good practice).
        *   Save the `MasterIndexStorage` via `es.save_master_index().await?`.
    *   [ ] **Cleanup:** Remove unused variables, imports (stream, mpsc, retry logic specific to old implementation), and constants (`RESERVATION_CONCURRENCY`, `UPLOAD_CONCURRENCY` might move into allocator). Remove `PendingScratchpadInfo` and related logic if no longer used by allocator.
    *   [ ] **Event Handling:** `store_item` emits `ConfirmReservation`. `allocate` emits `ReservationProgress`. `write` emits `StartingUpload`, `UploadProgress`, `ScratchpadCommitComplete`, and potentially `StoreComplete` (or `store_item` emits `StoreComplete` after successful `write` and index update).

3.  **Modify `MemoryAllocator`:** (`anthill-lib/src/allocator/allocator.rs`, `map.rs`, potentially `init.rs`)
    *   [ ] **Add `check_allocation_possible` Method:**
        *   Implement `pub async fn check_allocation_possible(&self, size: usize) -> Result<Option<usize>, Error>`.
        *   Locks map, checks if allocation is possible without expansion.
        *   If no, calculates `needed_pads` for expansion and returns `Ok(Some(needed_pads))`.
        *   If yes, returns `Ok(None)`.
    *   [ ] **Refactor `MemoryAllocator::allocate` / `MemoryMap::expand`:**
        *   Modify `expand` to perform reservations concurrently (Step 1). Use retry logic for API calls.
        *   Ensure `expand` updates `MemoryMap` state (`total_scratchpads`, block info) correctly.
        *   Ensure `expand` persists the updated `MemoryMap` state *after* successful concurrent reservations.
        *   Ensure `expand` emits `ReservationProgress` via the callback.
    *   [ ] **Refactor `MemoryAllocator::write`:**
        *   **Concurrency Requirement:** Implement concurrent writing logic (e.g., `JoinSet`/semaphores with `UPLOAD_CONCURRENCY` limit) similar to the V3 `upload_future`.
        *   Internally determine which scratchpads correspond to the `data_id` and the data chunks for each.
        *   For each pad/chunk: upload data (retry), commit (retry).
        *   **Event Requirement:** Emit `StartingUpload` before starting writes. Emit `UploadProgress` based on aggregated bytes written. Emit `ScratchpadCommitComplete` after each successful commit. Emit `StoreComplete` (or return success allowing `store_item` to emit it) after all writes/commits succeed.
        *   Use the passed `callback` for event emission.
        *   Handle persistence of any internal allocator state changes related to writing if necessary.

4.  **Review `PutEvent` & CLI Callback:** (`anthill-lib/src/events.rs`, `anthill-cli/src/callbacks/put.rs`)
    *   [ ] Verify events emitted by the refactored `allocate` and `write` methods align with the sequence expected by the CLI callback for the three progress bars.
    *   [ ] Ensure event data (`current`, `total`, `bytes_written`, `total_bytes`) is correct based on the allocator's internal progress. 