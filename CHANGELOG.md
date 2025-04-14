# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - Unreleased

### Changed
- **Refactor:** Split `MutAnt` initialization Step 4 (Load/Create Master Index) into sub-steps for clearer progress reporting. Creating the remote index (if prompted) is now a distinct Step 5.
- **CLI:** Reworked `put` progress display. The upload bar now shows "Upload complete. Committing..." when byte transfer finishes but remains visible. The commit bar progresses concurrently. Both bars are cleared upon final `StoreComplete` event.
- **Performance:** Refactored `MutAnt` initialization to be lazy. Network client initialization and remote master index fetching are now deferred until the first network operation (e.g., `put`, `get`, `sync`, `import`), making local cache-only operations like `ls` and `stats` significantly faster to start.
- Added a 5-second delay **after** static storage write operations (create/update) complete and **before** their verification loops begin.
- Modified `load_master_index_storage_static` in `mutant-lib` to automatically create and save a default Master Index Storage on the network if it's not found, empty, or fails to deserialize.
- Fixed an issue where the reservation progress bar could be cleared prematurely, causing warnings.

### Added
- Differentiate local index cache based on network choice (Devnet vs Mainnet). Cache files are now stored in `~/.local/share/mutant/{devnet,mainnet}/index.cbor`.
- Add `--push-force` flag to `sync` command to overwrite the remote master index with the local cache.
- `mutant import <private_key>` command to add an existing free scratchpad (specified by its private key hex) to the `free_pads` list in the Master Index. This allows transferring or recovering unused pads.
- `mutant sync` command to synchronize the local index cache with the remote index. Added a `--push-force` flag to overwrite the remote index with the local cache.
- Start of development for v0.1.1
- Added `reset` command to `mutant-cli` to clear the master index. Requires explicit confirmation by typing 'reset'.
- **Configuration:** Added `MutAntConfig` struct and `NetworkChoice` enum (Devnet/Mainnet) to allow configuring the MutAnt instance upon initialization. The `init` method now uses default configuration, while `init_with_progress` accepts a `MutAntConfig`.
- **Documentation:** Added a "Setup" section to `README.md` explaining the `ant wallet import` prerequisite.
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
- Fixed a data corruption issue during `get` caused by the list of storage pads (`PadInfo`) not being saved in the correct chunk order during `put`/`update`. The `perform_concurrent_write_standalone` function now collects `(index, PadInfo)` pairs, sorts them by index, and stores the correctly ordered list in the master index.
- Adapt tests in `mutant-lib/tests/mutant_test.rs` to handle the updated return signature of `MutAnt::init_with_progress` (no longer returns a tuple).

### Removed
- The specific changelog entry for updating the progress message to "Creating remote master index..." as this is now covered by the refactoring of init steps.

## [Unreleased]

### Changed
- Fix the commit progress bar to only track newly reserved pads.
- Ensure upload progress bar completion reflects data write finalization before commit steps.
- Emit UploadProgress/ScratchpadCommitComplete events from storage layer to accurately reflect put/get timing.

### Added
- `mutant list-details` command to show more information about stored keys.