# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Display completion percentage for incomplete keys in `ls` output (e.g., `*mykey (50.0%)`).
- Added detailed statistics section for incomplete uploads in `stats` output.
- Implemented resumable `put` operations. Uploads can now be interrupted and resumed, picking up from the last successfully written chunk.
- Introduced granular pad statuses (`Generated`, `Written`, `Confirmed`) in the index to track chunk progress accurately.
- Optimized `put` operation for newly generated pads by skipping unnecessary network existence checks before the initial write.

### Changed
- Refactored pad state management: `pending_verification_pads` in the index is now only used for pads associated with removed keys whose write status was uncertain (`Generated`).
- `purge` command is now the sole mechanism responsible for verifying pads in the `pending_verification_pads` list and moving them to the free pool.
- **Refactored `put` operation in `mutant-lib` to perform chunk writes and network confirmations concurrently, improving upload throughput.**
- **CLI:** Renamed "Reserving pads..." progress message during `put` to "Acquiring pads..." for clarity when reusing free pads.
- **CLI:** Refactored `put` progress display to use two bars:
  - "Creating pads...": Increments on `ChunkWritten` (network create/update success).
  - "Confirming pads...": Increments on `ChunkConfirmed` (network check success).
- Modified the `purge` command logic to only discard pending pads if the network explicitly returns a "Record Not Found" status. Pads encountering other network errors during verification are now returned to the pending list for future retries.
- **BREAKING**: `MutAnt::init` now returns `Result<Self, Error>` instead of `Result<Option<Self>, Error>`.
  The `Option` was redundant as failure modes are covered by the `Error` enum.
- Refactored internal layers for better separation of concerns (Network, Storage, Index, PadLifecycle, Data).
- Pad Lifecycle Management (`PadLifecycleManager`) now explicitly handles pad acquisition (from pool or new reservation) and state transitions.
- Data Operations (`DataManager`) orchestrate storage, index updates, and pad lifecycle changes.
- Store confirmation (`put`) now verifies by fetching and attempting decryption, not just checking existence.
- **Refactor:** `reserve` command logic (`reserve_pads` function) no longer attempts network creation via `put_raw`. It now only generates the key/address and adds the pad metadata to the index manager's free list, deferring network interaction until data is actually written.

### Fixed
- Corrected pad lifecycle management to ensure pads from cancelled or failed `put` operations are not prematurely released or discarded, allowing for proper resumption.
- **Fixed `put` progress reporting by ensuring the `Starting` event is emitted *before* pad acquisition begins, allowing the reservation progress bar to initialize correctly.**
- **Fix:** Prevent redundant existence check in `put_raw` when reusing free pads.
- **CLI:** Corrected `put` progress bars:
  - Fixed duplicate "Acquiring pads..." message.
  - "Acquiring pads..." bar now completes instantly.
  - "Writing chunks..." bar increments on `ChunkWritten` event.
  - "Confirming writes..." bar increments on `ChunkConfirmed` event.
- Corrected timing of `PadReserved` event emission during `put` operations. The event is now triggered within the write task completion handler in `DataOps`, *after* a successful network write (which implies pad reservation/creation), ensuring the progress bar accurately reflects confirmed pad reservations.
- Progress bars during resumed `put` operations now correctly initialize to show the previously completed progress.
- **CLI:** Redirect progress bar output to `stdout` to prevent interference with logs written to `stderr`.
- Handle `NotEnoughCopies` network errors during `put` resume existence checks. Instead of aborting or retrying the same pad, the operation now acquires a *new* replacement pad, adds the original problematic pad to the `pending_verification_pads` list for later checking (e.g., via `purge`), updates the index to use the new pad for the corresponding chunk, logs a warning, and proceeds with the operation. The index cache is saved immediately after the replacement to persist the state.
- Potential issue where empty scratchpad creation during reservation might lead to downstream errors.
- Store operation confirmation was insufficient, potentially marking data as confirmed even if it was corrupted or undecryptable.
- Payment failure errors during `reserve` caused by attempting premature network creation with empty data.

### Removed
- Removed redundant pad release functions (`pad_lifecycle::pool::release_pads_to_free`, `pad_lifecycle::manager::release_pads`) as the logic is now handled within `IndexManager` and `purge`.

## [0.2.0] - 2025-04-15

### Added
- Initial release with core functionality: put, get, ls, rm, update.
- Devnet support.
- Progress bars for long operations.
- Local index caching.
- Basic CLI structure and logging.
- New `mutant reserve` command to pre-allocate empty scratchpads.
- Progress bars for `put`, `get`, `purge` operations.
- Initialization progress reporting.
- `Mutant::import_free_pad` to import external pads.
- `Mutant::purge` to verify pending pads.
- `Mutant::save_master_index` and `Mutant::reset`.
- Synchronization commands (`mutant sync`).

### Fixed
- Store operation now saves the updated index to the local cache, ensuring `ls` reflects recent changes.
- Remove operation (`rm`) now saves the updated index to the local cache, ensuring `ls` reflects the removal.
- Initialization now proceeds with an empty in-memory index if the persisted index (local cache or remote) fails to deserialize (e.g., due to corruption or format changes), preventing errors during startup and allowing commands like `sync --push-force` to fix the remote index.

## [0.1.2] - Unreleased

### Changed
- **Initialization:** When prompting to create a remote index (if local cache and remote index are missing), declining the prompt no longer aborts initialization. Instead, it skips remote index creation and proceeds to create the local index cache only.
- **Refactor:** Split `MutAnt` initialization Step 4 (Load/Create Master Index) into sub-steps for clearer progress reporting. Creating the remote index (if prompted) is now a distinct Step 5.
- **CLI:** Reworked `put` progress display.
  - The `StartingUpload` event now includes `total_pads`.
  - The CLI callback uses this event field to initialize the confirmation bar's total count immediately, ensuring it shows `[0/N]` from the start even when no new pads are reserved (update-only scenario).
  - The upload bar now shows "Upload complete. Committing..." when byte transfer finishes but remains visible.
  - The commit bar progresses concurrently.
  - Both bars are cleared upon final `StoreComplete` event.
- **Performance:** Refactored `MutAnt` initialization to be lazy. Network client initialization and remote master index fetching are now deferred until the first network operation (e.g., `put`, `get`, `sync`, `import`), making local cache-only operations like `ls` and `stats` significantly faster to start.
- Added a 5-second delay **after** static storage write operations (create/update) complete and **before** their verification loops begin.
- Modified `load_master_index_storage_static` in `mutant-lib` to automatically create and save a default Master Index Storage on the network if it's not found, empty, or fails to deserialize.
- Fixed an issue where the reservation progress bar could be cleared prematurely, causing warnings.
- **CLI:** Refactored `sync` command progress display to use a single step-based progress bar instead of multiple spinners.
- **CLI:** Mark incomplete uploads with `*` in `mutant ls` output (both standard and long formats).
- **Sync:** Fixed an issue where the `sync` command (and other operations that save the index) would fail if the remote master index scratchpad did not exist. The save logic now checks for existence and calls the appropriate create or update function.
- Fixed a data corruption issue during `get` caused by the list of storage pads (`PadInfo`) not being saved in the correct chunk order during `put`/`update`. The `perform_concurrent_write_standalone` function now collects `(index, PadInfo)` pairs, sorts them by index, and stores the correctly ordered list in the master index.
- Network initialization is now lazy: Connects only if the local index cache is missing and remote index fetch is needed.

### Added
- Differentiate local index cache based on network choice (Devnet vs Mainnet). Cache files are now stored in `~/.local/share/mutant/{devnet,mainnet}/index.cbor`.
- Add `--push-force` flag to `sync` command to overwrite the remote master index with the local cache.
- `mutant import <private_key>` command to add an existing free scratchpad (specified by its private key hex) to the `free_pads` list in the Master Index. This allows transferring or recovering unused pads.
- `mutant sync` command to synchronize the local index cache with the remote index. Added a `--push-force` flag to overwrite the remote index with the local cache.
- Start of development for v0.1.1
- Added `reset` command to `mutant-cli` to clear the master index. Requires explicit confirmation by typing 'reset'.
- **Configuration:** Added `MutAntConfig` struct and `NetworkChoice` enum (Devnet/Mainnet) to allow configuring the MutAnt instance upon initialization. The `init` method now uses default configuration, while `init_with_progress` accepts a `MutAntConfig`.
- **Documentation:** Added a "Setup" section to `README.md` explaining the `ant wallet import` prerequisite.
- **Documentation:** Added `