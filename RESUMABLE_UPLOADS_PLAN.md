# Resumable Upload Feature Plan

This document outlines the plan to implement resumable uploads in the `mutant-lib`.

**Phase 1: Data Structure Modification** - **Completed**

*   **Observations:**
    *   The `PadUploadStatus` enum and `PadInfo` struct were added as planned.
    *   The `KeyStorageInfo` struct was updated to use `Vec<PadInfo>` for the `pads` field.
    *   The `data_checksum` field was added to `KeyStorageInfo`.
    *   The implementation followed the plan closely.

1.  **Define Pad Status:**
    *   In `mutant-lib/src/mutant/data_structures.rs`, introduce a new `enum PadUploadStatus`.
    *   Proposed States:
        *   `Generated`: Pad metadata created (key generated or reused), attempting initial write+confirmation.
        *   `Free`: Pad confirmed created/reserved on the network, awaiting data write+confirmation.
        *   `Populated`: Pad confirmed written with correct data chunk.
    *   ```rust
        #[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
        pub enum PadUploadStatus {
            Generated,  // Pad metadata known, initial write+confirm pending
            Free,       // Pad reservation confirmed, data write+confirm pending
            Populated,  // Data chunk confirmed written to this pad
        }
        ```

2.  **Define Pad Information:**
    *   In `mutant-lib/src/mutant/data_structures.rs`, create a new struct `PadInfo` to hold details for each scratchpad.
    *   ```rust
        use autonomi::ScratchpadAddress;
        use serde::{Deserialize, Serialize};

        // Assuming PadUploadStatus is defined above
        #[derive(Serialize, Deserialize, Debug, Clone)]
        pub struct PadInfo {
            pub address: ScratchpadAddress,
            pub key: Vec<u8>,
            pub status: PadUploadStatus,
            pub is_new: bool, // Flag to indicate if this pad needs creation vs update
            // Optional: pub chunk_index: usize, // If order matters and isn't implied by Vec index
        }
        ```

3.  **Update Key Storage Info:**
    *   In `mutant-lib/src/mutant/data_structures.rs`, modify the `KeyStorageInfo` struct.
    *   Change the `pads` field from `Vec<(ScratchpadAddress, Vec<u8>)>` to `Vec<PadInfo>`.
    *   Add `data_checksum: String` field.
    *   ```rust
        // ... existing imports ...
        use crate::mutant::data_structures::{PadInfo, PadUploadStatus}; // Adjust import as needed

        #[derive(Serialize, Deserialize, Debug, Clone)]
        pub struct KeyStorageInfo {
            // pub pads: Vec<(ScratchpadAddress, Vec<u8>)>, // Old field
            pub pads: Vec<PadInfo>, // New field
            pub data_size: usize,
            pub data_checksum: String, // New field
            #[serde(default = "Utc::now")]
            pub modified: DateTime<Utc>,
        }
        ```

**Phase 2: Pad Manager Refactoring (`mutant-lib/src/pad_manager/`)** - **Completed**

*   **Observations:**
    *   Created `pad_manager::write::write_chunk` function as a wrapper around storage network calls (`create_scratchpad_static`, `update_scratchpad_internal_static`).
    *   Necessary imports were added (Storage, network module, SecretKey, Error).
    *   The `network` module in `storage` was made `pub(crate)` to allow access from `pad_manager`.
    *   Addressed a name collision by renaming the local type alias `PadInfo` in `write.rs` to `PadInfoAlias` and added a `TODO` for future refactoring.
    *   Reviewed `read.rs`, `delete.rs`, and `util.rs`.
    *   Updated `read.rs` (`get_pads_and_size`, `spawn_fetch_tasks`, `retrieve_data`) to work with `Vec<PadInfo>` instead of `Vec<(Address, Key)>`.
    *   Updated `delete.rs` (`release_pads`) to correctly extract `(Address, Key)` tuples from `Vec<PadInfo>` before adding them to `free_pads`.
    *   `util.rs` (`estimate_reservation`) did not require changes as it doesn't directly use `KeyStorageInfo.pads`.
    *   Identified a `TODO` in `write_chunk` regarding the `content_type` for data chunks.
    *   Identified a `FIXME` in `allocate_and_write` where the new `data_checksum` field is not being set in `KeyStorageInfo`.

1.  **Consolidate Write Operations (`write.rs`):**
    *   Create `write_chunk(storage: &Storage, address: ScratchpadAddress, key_bytes: &[u8], data_chunk: &[u8], is_new_pad: bool) -> Result<(), crate::error::Error>`.
    *   This function will act as a wrapper around the storage layer calls.
    *   Internally calls `crate::storage::network::create_scratchpad_static` (if `is_new_pad`) or `crate::storage::network::update_scratchpad_internal_static` (passing necessary context like wallet for creation).
    *   The storage layer functions handle their own confirmation loops. `write_chunk` simply returns the `Result` from the storage call.

2.  **Review/Update Other Modules (`read.rs`, `delete.rs`, `util.rs`):**
    *   Ensure compatibility with the new `PadInfo` struct within `KeyStorageInfo`. Minimal changes expected. **(Completed)**

**Phase 3: Mutant Core Logic Overhaul (`mutant-lib/src/mutant/`)** - **Completed**

*   **Observations:**
    *   Replaced `store_logic::store_item` with `store_logic::store_data`.
    *   Implemented entry/resume logic based on checksum comparison.
    *   Implemented Planning Phase: Calculates chunks, reuses free pads, generates new pads, creates `Vec<PadInfo>` with statuses (`Generated`, `Free`), updates `KeyStorageInfo` (including checksum), and persists `MasterIndexStorage` locally before upload.
    *   Implemented Upload Execution Phase (`execute_upload_phase`): Iteratively calls `pad_manager::write::write_chunk` for `Generated` and `Free` pads, updates status (`Generated` -> `Free` -> `Populated`), modifies `KeyStorageInfo.modified`, and persists `MasterIndexStorage` locally after each successful chunk operation.
    *   Added helper functions `calculate_checksum` and `chunk_data`.
    *   Corrected linter errors related to `delete_item` call and `PutEvent::UploadProgress` usage.
    *   Refactored `update_logic::update_item` to perform `delete_item` then `store_data` as planned, resolving build errors stemming from Phase 1 changes.
    *   Confirmed `remove_logic.rs` requires no changes as `pad_manager::delete::release_pads` already handles the necessary logic.
    *   Updated `mutant::mod.rs` to call `store_data` instead of `store_item`.
    *   Added `ContentType::DataChunk` to `storage/mod.rs`.
    *   Introduced `TODO`s in `store_logic.rs` for potential callbacks and retry logic in the upload phase.
    *   The temporary fix in `pad_manager/write.rs` (mapping `PadInfoAlias` to `PadInfo` with placeholder status/is_new) when calling `allocate_and_write` still exists. This function is now only called by the old `update_item` logic (which we just replaced) and potentially internal tests. The new `store_data` logic uses `write_chunk` directly.

1.  **Refactor `store_logic.rs` (`store_data` function):**
    *   **Entry Point & Resume Logic:**
        *   Load `MasterIndex`.
        *   Check if `user_key` exists.
        *   If exists: Calculate & Check `data_checksum`. If matches & not fully populated, find `PadInfo` entries with `Generated` or `Free` status and jump to "Upload Execution". If checksum/size differs, treat as update (delete+store).
        *   If not exists: Proceed to "Planning Phase".
    *   **Planning Phase:**
        1.  Calculate chunks needed (`chunk_data`).
        2.  Identify reusable `free_pads`.
        3.  Generate keys/addresses for new pads.
        4.  Create `Vec<PadInfo>`: Populate with reused pads (`is_new: false, status: Free`) and new pads (`is_new: true, status: Generated`).
        5.  Create/Update `KeyStorageInfo` with this `Vec<PadInfo>`, `data_size`, `data_checksum`, `modified`.
        6.  Persist the updated `MasterIndexStorage` locally **immediately** (`write_local_index`).
    *   **Upload Execution Phase (`execute_upload_phase`):**
        1.  Iterate through `KeyStorageInfo.pads` (looping until no more `Generated` or `Free` are found).
        2.  For each `pad_info` with `status: Generated`:
            *   Get corresponding data chunk.
            *   Call `pad_manager::write::write_chunk` using `pad_info.is_new` (true).
            *   On success: Update `pad_info.status` to `Free`, update `KeyStorageInfo.modified`, persist `MasterIndexStorage` locally.
            *   On failure: Handle retries/error out. Progress is saved due to persistence after each chunk.
        3.  For each `pad_info` with `status: Free`:
            *   Get corresponding data chunk.
            *   Call `pad_manager::write::write_chunk` using `pad_info.is_new` (false).
            *   On success: Update `pad_info.status` to `Populated`, update `KeyStorageInfo.modified`, persist `MasterIndexStorage` locally. Send `UploadProgress` event.
            *   On failure: Handle retries/error out. Progress is saved due to persistence after each chunk.
        4.  If all pads become `Populated`, upload is complete. Send `UploadFinished` event.

2.  **Refactor `update_logic.rs`:**
    *   Recommend treating updates as `delete` followed by `store` initially for simplicity. **(Completed)**

3.  **Refactor `remove_logic.rs`:**
    *   Ensure pads from the deleted `KeyStorageInfo` are returned to `free_pads` in `MasterIndexStorage`. **(Completed - No changes needed)**

**Phase 4: Testing and Final Steps**

1.  **Unit and Integration Tests:**
    *   Test data structure (de)serialization.
    *   Test `pad_manager::write::write_chunk`.
    *   Test `mutant::store_logic::store_data` extensively (new, reuse, mix, **resume scenarios**, updates, delete).
2.  **Build:** `cargo build --all-targets`. Fix errors.
3.  **Changelog:** Update `CHANGELOG.md`.
4.  **Commit:** `git commit -m "feat(mutant): implement resumable uploads"`. 