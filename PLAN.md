# Plan for Implementing Local Index Cache

This document outlines the steps to implement a local cache for the `MasterIndexStorage` in MutAnt.

**Goal:** Improve performance and reduce network usage for read-heavy operations by storing the index locally. Introduce a `sync` command to reconcile local and remote states.

**Cache Format:** CBOR (`serde_cbor`)
**Cache Location:** XDG Data Directory (`~/.local/share/mutant/index.cbor`)

---

## Progress Tracker

- [x] **Step 1: Setup & Dependencies**
- [x] **Step 2: Cache Utilities Implementation**
- [x] **Step 3: Modify Initialization Logic**
- [x] **Step 4: Implement `sync` Command**
- [x] **Step 5: Adjust Write Operations (`store`, `update`, `remove`)**
- [ ] **Step 6: Testing**
- [ ] **Step 7: Final Steps (Build, Changelog, Commit)**

---

## Detailed Steps

### Step 1: Setup & Dependencies

*   **Goal:** Add necessary dependencies to `mutant-lib/Cargo.toml`.
*   **Tasks:**
    *   Add `serde_cbor = "0.11"`
    *   Add `dirs = "5.0"`
*   **Verification:** Run `cargo build --package mutant-lib` to ensure dependencies resolve and the library compiles.
*   **Status:** Complete - Build successful after fixing `autonomi` dependency reference.

### Step 2: Cache Utilities Implementation (`mutant-lib/src/cache.rs`)

*   **Goal:** Create helper functions for reading and writing the index cache file using CBOR format in the appropriate XDG data directory.
*   **Tasks:**
    *   Create `mutant-lib/src/cache.rs`.
    *   Implement `get_cache_dir() -> Result<PathBuf, Error>`: Returns `~/.local/share/mutant/` using `dirs::data_dir()`. Creates the directory if it doesn't exist.
    *   Implement `get_cache_path() -> Result<PathBuf, Error>`: Returns the full path `~/.local/share/mutant/index.cbor` by calling `get_cache_dir()`.
    *   Implement `read_local_index() -> Result<Option<MasterIndexStorage>, Error>`: Reads `index.cbor`, deserializes using `serde_cbor::from_reader`. Returns `Ok(None)` if the file doesn't exist (use `std::fs::File::open` and check for `ErrorKind::NotFound`). Handles other file I/O and CBOR errors.
    *   Implement `write_local_index(index: &MasterIndexStorage) -> Result<(), Error>`: Serializes the index using `serde_cbor::to_writer` and writes it to `index.cbor` via `get_cache_path()`. Ensure atomicity if possible (write to temp file, then rename).
    *   Add `pub mod cache;` to `mutant-lib/src/lib.rs`.
*   **Verification:** Run `cargo build --package mutant-lib` to ensure the new module compiles.
*   **Status:** Complete - Build successful after adding `CacheError` enum variant and fixing duplicates.

### Step 3: Modify Initialization Logic (`mutant-lib/src/storage/mod.rs`)

*   **Goal:** Change the `storage::new` function to attempt loading the index from the local cache before fetching it remotely.
*   **Tasks:**
    *   Inside `storage::new`, before the remote fetch logic:
        *   Call `crate::cache::read_local_index()`.
        *   If `Ok(Some(cached_index))`:
            *   Initialize the `Arc<Mutex<MasterIndexStorage>>` with `cached_index`.
            *   Log that the cache was successfully loaded.
            *   Skip the remote fetching logic for the index itself (but still initialize network, wallet etc.).
        *   If `Ok(None)` or `Err(_)`:
            *   Log that the cache was not found or was invalid.
            *   Proceed with the *existing* remote fetch logic.
            *   **Crucially:** After successfully fetching/creating the index remotely, call `crate::cache::write_local_index(&fetched_or_created_index)` to populate the cache for the next run.
*   **Verification:** Run `cargo build --package mutant-lib`. Observe logs during initialization to confirm cache loading/miss behavior.
*   **Status:** Complete - Build successful after fixing async/await issues and warnings.

### Step 4: Implement `sync` Command

*   **Goal:** Add a new CLI command `sync` to merge local and remote indices and persist the result both locally and remotely.
*   **Tasks:**
    *   **CLI Definition (`mutant-cli/src/cli.rs`):**
        *   Add a `Sync` variant to the `Commands` enum.
        *   Add `#[command(about = "Synchronize local index cache with remote storage")] Sync,`
    *   **Remote Fetch Method (`mutant-lib/src/storage/mod.rs` or `mutant-lib/src/mutant/mod.rs`):**
        *   Create a new *internal* function (e.g., `fetch_remote_master_index_internal`) within the `storage` module that *only* performs the remote fetch/deserialization logic (essentially, extract the remote-fetch part from the current `storage::new`). This function should *not* interact with the local cache.
        *   Expose this capability via a new public method on `MutAnt`, e.g., `pub async fn fetch_remote_master_index(&self) -> Result<MasterIndexStorage, Error>`. This method will call the internal storage function.
    *   **Command Logic (`mutant-cli/src/commands/sync.rs`):**
        *   Create `mutant-cli/src/commands/sync.rs`.
        *   Implement `pub async fn handle_sync(mutant: MutAnt) -> Result<(), CliError>`.
        *   Inside `handle_sync`:
            *   Call `cache::read_local_index()` to get the local state (`local_index`). Handle potential errors/missing cache (treat as empty default index).
            *   Call `mutant.fetch_remote_master_index()` to get the remote state (`remote_index`). Handle errors.
            *   Perform the merge:
                *   Clone the `remote_index` into `merged_index`.
                *   Iterate through `local_index.index`. For each `(key, local_info)`:
                    *   If `merged_index.index` does *not* contain `key`, insert `(key.clone(), local_info.clone())`.
                *   Merge `free_pads`: Combine `local_index.free_pads` and `remote_index.free_pads`, ensuring no duplicate addresses. (Simple concatenation followed by deduplication might work for now).
                *   Decide on `scratchpad_size`: Use the remote one? Log a warning if they differ? For now, let the remote value overwrite the local one during the initial clone.
            *   Update in-memory state: Lock `mutant.master_index_storage` and replace its content with `merged_index`.
            *   Persist locally: Call `cache::write_local_index(&merged_index)`. Handle errors.
            *   Persist remotely: Call `mutant.save_master_index()`. Handle errors. (Note: `save_master_index` saves the current *in-memory* state, which we just updated).
            *   Print status message to the user.
    *   **Connect Handler (`mutant-cli/src/app.rs`):**
        *   Add `mod sync;` to `commands/mod.rs`.
        *   In `run_cli()`, add a match arm for `Commands::Sync { .. } => commands::sync::handle_sync(mutant).await,`.
*   **Verification:** Run `cargo build`. Manually test the `sync` command under different scenarios (empty cache, cache == remote, cache != remote).
*   **Status:** Complete - Build successful after refactoring command handlers and implementing sync logic (CLI connection pending).

### Step 5: Adjust Write Operations (`store`, `update`, `remove`)

*   **Goal:** Modify `store`, `update`, and `remove` operations to update the *local cache* immediately after modifying the in-memory index, but *not* save to the remote backend.
*   **Tasks:**
    *   Locate the points in `mutant-lib/src/mutant/store_logic.rs`, `update_logic.rs`, and `remove_logic.rs` where `self.save_master_index().await` is currently called *after* successfully modifying the `master_index_storage`.
    *   **Remove** the call to `self.save_master_index().await`.
    *   **Instead**, after the in-memory `master_index_storage` is successfully modified (and the lock is released), call `crate::cache::write_local_index(&*mis_guard).await` (or similar, passing the updated index). Handle potential errors during the cache write (log them, but the primary operation might still be considered successful).
*   **Verification:** Run `cargo build`. Test `put`, `rm`, `up` commands and verify that `index.cbor` is updated, but the remote index (checked via a subsequent `sync` or manual inspection) is not immediately changed.
*   **Status:** Complete - Build successful after modifying write logic.

### Step 6: Testing

*   **Goal:** Add unit and integration tests for the new caching and sync logic.
*   **Tasks:**
    *   Unit tests for `cache.rs` functions (reading, writing, error handling).
    *   Unit tests for the merge logic within the `sync` implementation (can be extracted into a testable function).
    *   Integration tests (`tests/` directory):
        *   Test `init` with empty cache -> fetches remote, creates cache.
        *   Test `init` with existing cache -> loads from cache.
        *   Test `put`/`rm`/`up` -> modifies cache, doesn't modify remote immediately.
        *   Test `sync` with various states (local only changes, remote only changes, conflicting changes - though conflict resolution is basic now).
        *   Test `ls`/`stat`/`get` -> read correctly from the cache after modifications and syncs.
*   **Verification:** Run `cargo test`. All tests should pass.
*   **Status:** Not Started

### Step 7: Final Steps (Build, Changelog, Commit)

*   **Goal:** Ensure the project builds cleanly, update documentation, and commit the feature.
*   **Tasks:**
    *   Run `cargo build --all-targets` in the workspace root. Fix any errors/warnings.
    *   Update `CHANGELOG.md` with details about the new local caching feature and `sync` command.
    *   Stage all changes (`git add .`).
    *   Commit the changes (`git commit -m "feat: Implement local index caching and sync command"`), ensuring the command runs in the correct directory (`/home/champii/prog/rust/Anthill`).
*   **Verification:** Build completes successfully. Changelog is updated. Git status is clean.
*   **Status:** Not Started 