# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
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

### Fixed
- Corrected pad lifecycle management to ensure pads from cancelled or failed `put` operations are not prematurely released or discarded, allowing for proper resumption.
- **Fixed `put` progress reporting by ensuring the `Starting` event is emitted *before* pad acquisition begins, allowing the reservation progress bar to initialize correctly.**
- **Fix:** Prevent redundant existence check in `put_raw` when reusing free pads.
- **CLI:** Corrected `put` progress bars:
  - Fixed duplicate "Acquiring pads..." message.
  - "Acquiring pads..." bar now completes instantly.
  - "Writing chunks..." bar increments on `ChunkWritten` event.
  - "Confirming writes..." bar increments on `ChunkConfirmed` event.
- Corrected timing of `PadReserved` event emission during `put` operations. The event is now triggered after the pad acquisition phase completes in `DataOps`, rather than prematurely during pad generation in `PadLifecycleManager`. This ensures the "Acquiring pads..." progress bar reflects the actual reservation process more accurately.

### Removed
- Removed redundant pad release functions (`pad_lifecycle::pool::release_pads_to_free`, `pad_lifecycle::manager::release_pads`) as the logic is now handled within `IndexManager` and `purge`.

## [0.2.0] - 2025-04-15

### Added
- Initial release with core functionality: put, get, ls, rm, update.
- Devnet support.
- Progress bars for long operations.
- Local index caching.
- Basic CLI structure and logging.

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
- **Documentation:** Added `antup` installation instructions to the "Setup" section in `README.md`.
- **Documentation:** Clarified in `README.md` that users can also create a new wallet with `ant wallet create`.
- **Documentation:**
    - Replaced outdated `docs/memory_allocator_design.md` with a new set of detailed documentation for `mutant-lib`:
        - `docs/mutant_lib/overview.md`: High-level architecture and concepts.
        - `docs/mutant_lib/internals.md`: Detailed explanation of data structures, component logic (PadManager, Storage), error handling, and concurrency.
        - `docs/mutant_lib/usage.md`: Practical API usage examples.
- `--version` flag to display the crate version.
- Basic `status` command to show MutAnt and Storage status.
- Basic `storage nodes` command to list known storage nodes.
- `--local` flag to connect to local devnet instead of mainnet.
- Improved wallet handling:
  - Removed `--wallet-path` argument.
  - Added config file (`~/.config/mutant/mutant.json`) to store selected wallet path.
  - Scans standard Autonomi wallet directory (`$XDG_DATA_HOME/autonomi/client/wallets`).
  - Prompts user to select a wallet if multiple are found or config is missing.
  - Reads private key directly from the selected wallet file.
- Added a disclaimer to the README regarding production readiness.
- Verification loop with 1-hour timeout for static scratchpad create and update operations to ensure data propagation before returning.
- Implemented local caching for the master index (`~/.local/share/mutant/index.cbor`).
- Reads/writes use the local cache first, reducing network requests for most operations.
- Added a new `sync` command to reconcile the local cache with the remote master index.
- Write operations (`put`, `rm`, `update`) now update the local cache immediately and require `sync` to push changes remotely.
- Show completion percentage for incomplete uploads in `mutant ls` output (both standard and long formats).
- **CLI:** Added progress spinners to the `sync` command for better visual feedback during network operations (fetching/saving index) and local processing (reading/writing cache, merging).

### Changed
- **`mutant-lib` API:** Unified key parameter types in `store`, `store_with_progress`, `update`, and `update_with_progress` to accept `&str` instead of `String` for consistency.
- **`mutant-lib` API:** Refactored methods (`init`, `store`, `fetch`, `update`) to have two variants:
    - Standard methods (e.g., `store`) which do not take a progress callback.
    - Methods with a `_with_progress` suffix (e.g., `store_with_progress`) which optionally accept a callback function for detailed progress reporting.
- **`mutant-lib` API:** Modified `MutAnt::init` and `MutAnt::init_with_progress` to only require the `private_key_hex` string. The `autonomi::Wallet` is now derived internally.
- Renamed `anthill-cli` to `mutant-cli` internally and directory structure.
- **Documentation:** Simplified usage example in `docs/mutant_lib/usage.md` by removing callbacks and progress reporting details.
- **Documentation:** Removed all `println!` calls from the `run_mutant_examples` function in `docs/mutant_lib/usage.md`.
- Updated dependencies.
- Reservation and data upload during `put` operation now run concurrently. Successfully reserved pads are immediately made available for upload via an internal channel, improving performance for multi-pad uploads.
- Pad reservation now saves the master index after *each* individual pad is successfully reserved and added to the free list, increasing robustness against failures during reservation but potentially impacting performance.
- Removed 10-second delay from scratchpad verification loops, allowing faster retries.

### Fixed
- Ensure CLI errors are always printed to stderr in `main`
- **CLI:** Suspend progress bar drawing during interactive prompts to prevent display corruption (e.g., when asking to create a remote index).
- **CLI:** Ensure initialization progress correctly advances to Step 5/6 when creating the remote master index after prompt confirmation.
- **CLI:** Mark incomplete uploads with `*` in `mutant ls` output (both standard and long formats).
- Fixed a data corruption issue during `get` caused by the list of storage pads (`PadInfo`) not being saved in the correct chunk order during `put`/`update`. The `perform_concurrent_write_standalone` function now collects `(index, PadInfo)` pairs, sorts them by index, and stores the correctly ordered list in the master index.
- Adapt tests in `mutant-lib/tests/mutant_test.rs` to handle the updated return signature of `MutAnt::init_with_progress` (no longer returns a tuple).
- Fixed confirmation progress bar counting. The CLI callback now uses the `current` value from the `PadConfirmed` event directly, resolving an issue where the internal counter was incrementing beyond the actual total due to duplicate event emissions or double counting.
- Fixed upload progress bar logic in `put` callback. Switched the internal aggregate byte counter in `mutant-lib` from `Arc<Mutex<u64>>` to `Arc<AtomicU64>` to ensure correct and consistent progress reporting in the `UploadProgress` event, even with highly concurrent pad updates. The CLI callback now reliably reflects the total uploaded bytes.
- Handle "Scratchpad already exists" errors gracefully during pad creation in `create_scratchpad_static`.
  This prevents failures when resuming an interrupted upload where some pads were created but not yet marked as completed locally.
- Handle `RecordNotFound` error when attempting to update a reused scratchpad (`is_new: false`) that doesn't actually exist on the network. The `write_chunk` function now checks for the pad's existence and attempts to create it if it's missing, preventing hangs during `put` operations involving reused pads.
- Prevent fetching data for keys with incomplete uploads (`mutant get`). A specific error message is shown.
- **Sync:** The `sync` command now correctly handles the case where the remote master index does not exist (e.g., after initializing with `--local` or declining the initial remote creation prompt). Instead of erroring, it creates the remote index based on the current `MutAnt` state before proceeding with the sync.

### Removed
- The specific changelog entry for updating the progress message to "Creating remote master index..." as this is now covered by the refactoring of init steps.

## [0.1.1] - 2024-04-10

### Fixed
- Resolved hangs during scratchpad creation/update verification loops by adding timeouts and improving retry logic in `storage::network`.
- Fixed panic in `sync` command when local cache exists but remote index doesn't.
- Corrected free pad merging logic in `sync` command to avoid losing track of pads.
- Ensured Master Index `scratchpad_size` is initialized correctly on first run.

### Changed
- Improved progress reporting during initialization and sync.
- Enhanced logging across library and CLI.

## [0.1.0] - Initial Release

- Core `store`, `fetch`, `remove`, `update` functionality.
- Basic CLI commands (`put`, `get`, `rm`, `up`, `list`).
- Local caching of Master Index.
- Initial pad management logic (no reuse).

### Changed

- **Rewritten progress reporting for `