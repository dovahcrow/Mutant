# Storage Simplification Refactor Plan

This plan outlines the steps to refactor the storage mechanism, replacing the complex `MemoryAllocator` with a simpler `PadManager` that maps keys directly to scratchpads and implements pad recycling.

**Goal:** Reduce complexity, improve reliability, and maintain concurrency.

**Strategy:** Introduce a `PadManager` responsible for pad allocation, writing, reading, and recycling. Minimize lock contention on the master index by using a temporary reservation strategy.

---

## Phase 1: New PadManager & Data Structures

*   [X] **Step 1: Define Core Data Structures:**
    *   Location: `anthill-lib/src/anthill/data_structures.rs`
    *   Tasks:
        *   Define `KeyStorageInfo { pads: Vec<(autonomi::ScratchpadAddress, autonomi::SecretKey)>, data_size: usize }`.
        *   Modify `MasterIndexStorage`:
            *   Change `index: HashMap<String, DataLocation>` to `index: HashMap<String, KeyStorageInfo>`.
            *   Add `free_pads: Vec<(autonomi::ScratchpadAddress, autonomi::SecretKey)>`.
            *   Add `scratchpad_size: usize`.
            *   Remove `pending_uploads`, `pending_reservations`.
        *   Remove the `DataLocation` enum.
        *   Update `serde` implementations as needed.
    *   Location: `anthill-lib/src/`
    *   Tasks:
        *   Create directory `pad_manager/`.
*   [X] **Step 2: Implement `PadManager` Struct & `new`:**
    *   Location: `anthill-lib/src/pad_manager/mod.rs`
    *   Tasks:
        *   Define `PadManager { storage: Arc<BaseStorage>, master_index_storage: Arc<Mutex<MasterIndexStorage>> }`.
        *   Implement `PadManager::new(...)`.
*   [ ] **Step 3: Implement `PadManager` Core Logic (Write/Update):**
    *   Location: `anthill-lib/src/pad_manager/write.rs` (or similar)
    *   Tasks:
        *   Implement private `acquire_resources_for_write(...)` helper (runs under lock).
        *   Implement public `allocate_and_write(...)`:
            *   Brief lock to call `acquire_resources_for_write`.
            *   Perform reservation & writing without lock.
            *   Brief lock to finalize/commit state or cleanup on failure.
*   [ ] **Step 4: Implement `PadManager::retrieve_data`:**
    *   Location: `anthill-lib/src/pad_manager/read.rs` (or similar)
    *   Tasks:
        *   Implement `retrieve_data(key)`. Brief lock to get pad info, unlock for reads.
*   [ ] **Step 5: Implement `PadManager::release_pads`:**
    *   Location: `anthill-lib/src/pad_manager/delete.rs` (or similar)
    *   Tasks:
        *   Implement `release_pads(key)`. Brief lock to move pads to `free_pads`.
*   [ ] **Step 6: Implement `PadManager::estimate_reservation`:**
    *   Location: `anthill-lib/src/pad_manager/util.rs` (or similar)
    *   Tasks:
        *   Implement `estimate_reservation(data_size)`. Brief lock to check `free_pads`.

## Phase 2: Integration & Refactoring

*   [ ] **Step 7: Refactor `store_item`:**
    *   Location: `anthill-lib/src/anthill/store_logic.rs`
    *   Tasks: Replace allocator logic with calls to `PadManager`. Handle confirmations. Save index.
*   [ ] **Step 8: Implement `update_item`:**
    *   Location: `anthill-lib/src/anthill/update_logic.rs` (create if needed)
    *   Tasks: Similar to `store_item`, but ensures key exists. `PadManager::allocate_and_write` handles update internally. Save index.
*   [ ] **Step 9: Refactor `retrieve_item`:**
    *   Location: `anthill-lib/src/anthill/retrieve_logic.rs` (or in `anthill.rs`)
    *   Tasks: Replace allocator logic with call to `PadManager::retrieve_data`.
*   [ ] **Step 10: Refactor `delete_item`:**
    *   Location: `anthill-lib/src/anthill/delete_logic.rs` (or in `anthill.rs`)
    *   Tasks: Replace allocator logic with call to `PadManager::release_pads`. Save index.
*   [ ] **Step 11: Update `Anthill` Struct & Initialization:**
    *   Location: `anthill-lib/src/anthill/mod.rs`
    *   Tasks:
        *   Remove `allocator` field, add `pad_manager` field.
        *   Update `Anthill::new` to handle modified `MasterIndexStorage` load/save and initialize `PadManager`.

## Phase 3: Cleanup & Testing

*   [X] **Step 12: Remove Old Allocator:**
    *   Tasks: Delete `anthill-lib/src/allocator/`. Remove `use` statements and `DataId`.
*   [ ] **Step 13: Update Tests:**
    *   Tasks: Adapt existing tests. Add tests for pad recycling, updates (shrink/grow), failure recovery.
*   [ ] **Step 14: Documentation & Changelog:**
    *   Tasks: Update code comments, README. Add detailed `CHANGELOG.md` entry.
*   [ ] **Step 15: Commit:**
    *   Tasks: Commit the completed refactor.

---
## Progress & Observations

*   **Current Step:** Phase 3, Step 13
*   **Observations:**
    *   Successfully modified `data_structures.rs`.
    *   Successfully created `pad_manager` directory.
    *   Successfully defined `PadManager` struct and `new` method in `pad_manager/mod.rs`.
    *   Implemented core write/update logic in `pad_manager/write.rs`.
    *   Implemented read logic in `pad_manager/read.rs`.
    *   Implemented delete logic in `pad_manager/delete.rs`.
    *   Implemented estimate logic in `pad_manager/util.rs`.
    *   Refactored `store_item` in `store_logic.rs` to use `PadManager`.
    *   Implemented `update_item` in `update_logic.rs` using `PadManager`.
    *   Refactored `retrieve_item` (was `fetch_item`) in `fetch_logic.rs` to use `PadManager`.
    *   Refactored `delete_item` (was `remove_item`) in `remove_logic.rs` to use `PadManager`.
    *   Updated `Anthill` struct and `Anthill::new` in `anthill/mod.rs`.
    *   Declared `pad_manager` module in `lib.rs`.
    *   Removed old `allocator` module directory and code references (`DataId`, `Error::InvalidDataId`).
    *   **BLOCKER:** Linter errors remain in `storage.rs` due to inability to resolve `autonomi::KeyPair` import path after 3 attempts. Placeholder logic for key/address derivation is invalid. Need correct import path for `autonomi::KeyPair`.
    *   **BLOCKER:** `storage.rs` also missing dependency `ciborium` and uses non-existent `InitProgressEvent` variants.
    *   **TODO:** Callback support needs to be added to `PadManager::retrieve_data` or handled outside.
    *   **Note:** The write logic includes update handling (grow/shrink/same) internally.
*   **Plan Revisions:** Postponing full test execution until `storage.rs` issues are resolved.