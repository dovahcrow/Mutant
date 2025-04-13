# Progress Bar Synchronization Fix Plan

**Goal:** Emit `UploadProgress` immediately after a chunk's `put` succeeds and `ScratchpadCommitComplete` immediately after that chunk's `get` verification succeeds within the storage layer, respecting the internal `sleep`.

**Core Strategy:** Pass the callback and necessary progress-tracking state down into the function responsible for the verified write (`update_scratchpad_internal_static_with_progress`) and trigger the events from within that function at the correct points. Remove the old, delayed event triggers from the calling function (`spawn_write_task_standalone`).

**Implementation Steps:**

- [X] **1. Locate Target Storage Function:** Confirmed `update_scratchpad_internal_static` in `mutant-lib/src/storage/network.rs`.
- [X] **2. Modify Target Storage Function Signature:** Added parameters to `update_scratchpad_internal_static_with_progress`. Created wrapper `update_scratchpad_internal_static` for internal calls.
- [X] **3. Refactor Target Storage Function Internals:** Inserted logic to emit `PutEvent::UploadProgress` after `put` and `PutEvent::ScratchpadCommitComplete` after verified `get` within `update_scratchpad_internal_static_with_progress`.
- [X] **4. Check `retry_operation` Utility:** Confirmed `retry_operation` signature in `utils/retry.rs` is suitable for captured context.
- [X] **5. Update Call Site (`spawn_write_task_standalone`):** Modified the call within `retry_operation` to use `update_scratchpad_internal_static_with_progress`, passing captured context. Removed old event emission logic.
- [X] **6. Build, Test, and Verify:** Build successful. User verified fix.
- [ ] **7. Update Changelog & Commit:** Add entry to `CHANGELOG.md` and commit the changes. 