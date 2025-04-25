# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- Fix write pipeline deadlock: Ensure main pipeline tasks (`put_private_pads`, `confirm_private_pads`) don't hold sender handles needed by each other's receiver loops by passing clones into the tasks and removing explicit drops within them.
- Fix process hang by ensuring concurrent write/confirm tasks terminate using MPSC channels.
- Adapt to `scratchpad_put` SDK changes where it no longer returns a `Receipt`. Code now correctly destructures the `(AttoTokens, ScratchpadAddress)` tuple and avoids updating the `PadInfo.receipt` field, resolving compilation errors.
- Correctly fetch and store network counter for harvested pads added to the free list, ensuring accurate `initial_counter` for `PadOrigin::FreePool`.
- Initialize `PadInfo.last_known_counter` using the `initial_counter` from `PadOrigin` when preparing pads, ensuring accurate initial counter tracking for pads from the free pool.

### Changed
- Refactor write pipeline: Replaced two-stage put/confirm tasks with a single processing loop (`process_pads`) using `tokio::select!` and `FuturesUnordered` to manage concurrent `process_pad_task` operations (put -> confirm cycle) for each pad, improving deadlock resilience.
- Refactored pad preparation logic to use a single `match` statement for determining initial status and counter based on `PadOrigin`.
- Optimized resume preparation: Removed upfront concurrent `check_existence` calls for `Generated` pads. `put_raw` now attempts creation and falls back to update if the pad already exists.
- Refactored `mutant-lib::data::Data::put` method to extract private pad putting and confirmation logic into helper functions.
- Modified pad confirmation retry logic to use a total time budget instead of fixed retries and sleep.
- Refactored mutant-cli to use the new mutant-client library
- Implemented basic task list functionality in mutant-cli

### Added
- Added integration test (`test_generated_pad_counter_increment`) to verify scratchpad counter increments correctly on successive writes to generated pads.
- Initial implementation of `mutant-daemon`:
    - WebSocket server on `/ws`.
    - JSON-based request/response protocol.
    - Background task management system (UUID based).
    - Handles `Put` and `Get` requests (private, Medium mode only).
    - Handles `QueryTask` and `ListTasks` requests.
    - Basic error handling and reporting via WebSocket.

## [0.4.2] - UNRELEASED

### Fixed
- Fix process hang caused by non-terminating background tasks during data write operations.

## [0.4.1] - 2024-04-23

### Added
- **Fetch History:** Keep track of fetched public data addresses for easy re-fetching. This history is displayed with `mutant ls -l`.

## [0.4.0] - 2024-05-14

### Added
- **Public Uploads:**
    - Store public, unencrypted data using `mutant put -p <name> <value>` or `mutant.store_public(name, data)`.
    - Fetch public data by name (`mutant get <name>`) or by its scratchpad address (`mutant get -p <address>`).
    - Update public uploads using `mutant put -p <name> <new_value> --force`.
    - List public uploads alongside private keys using `mutant ls` (shows name) and `mutant ls -l` (shows name, type, size, modified date, and address).
- **Keyless Public Fetch:** Initialize `MutAnt` without a private key using `MutAnt::init_public().await?` or `MutAnt::init_public_local().await?` to fetch public data by address using `fetch_public(address, progress_hook)`. This is useful for applications that only need read access to public data.
- **Progress Bars:** Added progress bars for `put` and `get` operations in the CLI.

### Changed
- **BREAKING (Internal API):** Refactored storage interaction logic. The `store` and `fetch` operations now potentially use different underlying storage methods based on data size or type, though the high-level API remains largely the same for standard usage. Direct interaction with lower-level storage components might be affected.
- **BREAKING (Index Format):** The internal structure for storing public upload metadata has changed. While upgrades should ideally be seamless, issues might arise. If you encounter problems fetching previously stored *public* data after upgrading, you might need to resynchronize (`mutant sync`) or potentially remove the old index cache (`rm ~/.local/share/mutant/index_cache.*.cbor`) and force-push the index (`mutant sync --push-force`). Private data indexing remains unchanged.
- Improved output formatting for `mutant stats`.
- Updated `autonomi` dependency and other minor dependencies.
- Enhanced error handling and logging details.

### Fixed
- Prevented `store` from overwriting existing completed keys unless `--force` is used.
- Fixed `rm` command to return a `KeyNotFound` error when trying to remove a non-existent key.
- Addressed potential data corruption issues when updating public data scratchpads (worked around a suspected SDK bug).
- Resolved various minor bugs and compilation issues, particularly in integration tests.
- Corrected dummy private key generation for local network initialization.

## [0.3.0] - 2024-06-02

### Added
- Added comprehensive architecture & implementation documentation (`docs/mutant_lib/architecture.md`).
- Added crate-level documentation to `mutant-lib/src/lib.rs`.
- Added retry logic for transient errors (`NotEnoughCopies`, `Timeout`) during the initial write phase of `put`.
- Added integration tests for `DataManager` (store, fetch, remove) and chunking.
- Added integration tests for `PadLifecycleManager::reserve_pads` and `PadLifecycleManager::purge`.
- Added integration tests for `StorageManager` and `NetworkAdapter`.
- Introduced `PadStatus::Allocated` to explicitly track network existence of scratchpads before write confirmation.
- Introduced `NetworkError::InconsistentState` variant.
- Added retry loop to store confirmation counter check.
- Implemented counter check for store confirmation.
- Added `mutant reserve` command to pre-allocate empty scratchpads.
- Added progress bars for `reserve` command with incremental index saving.
- Added abandoned pad to pending list on `NotEnoughCopies` during `put`.
- Added display of incomplete upload details in `ls` and `stats` output.
- Implemented resumable `put` operations via index cache.
- Implemented lazy network initialization.
- Store cache index file in `$XDG_DATA_HOME/mutant` instead of local `.mutant` directory.
- Documentation comments to various modules (`index`, `network`, `data/ops`, `data/chunking`, `pad_lifecycle`).
- Add explicit build checks and error reporting for `evm-testnet` and `antctl` in `manage_local_testnet.sh`.
- Add diagnostics (log dump, ps, pgrep -af) to `start_evm` when `pgrep` fails to find process.
- Add check for `pgrep` command existence in `manage_local_testnet.sh`.
- Public upload functionality (`mutant put --public <name>`)
- Public fetch functionality (`mutant cat --public <name>`)
- Ability to initialize `mutant-lib` without a private key for public fetch.
- `--public` flag for `ls` and `stat` commands to show public upload details.

### Fixed
- Fixed doctests in `mutant-lib/src/lib.rs` by:
  - Correcting `MutAnt::init` and `MutAnt::store` call arguments.
  - Adding `anyhow` dependency and import.
  - Replacing placeholder private key with a dummy one.
  - Commenting out network-dependent operations (`store`, `fetch`).
- Corrected imports and types in index integration tests.
- Enhanced store confirmation logic to verify decrypted data size matches expected size.
- Treat `NotEnoughCopies` network error during store confirmation fetch as a retriable error.
- Prevent `mutant rm` from performing unnecessary network checks for non-`Generated` pads.
- Fixed various Clippy lints (`collapsible_else_if`, `useless_conversion`, `redundant_closure`, `clone_on_copy`, `doc_overindented_list_items`, `dead_code`).
- Corrected chunk size usage in `store_op` and added a 16MiB test.
- Correctly set `is_new_hint` based on pad origin (FreePool vs. Generated) to avoid unnecessary network calls.
- **Workaround:** Avoid calling `scratchpad_update` in `put_raw` due to suspected SDK bug causing data truncation.
- Fixed `rm` to use pad origin initial counter and skip network fetch.
- Fixed handling of `NotEnoughCopies` error during `put` resume.
- Fixed `purge` command to only discard pads on `RecordNotFound`, retrying on other errors.
- Fixed CLI progress bars drawing to `stdout` to avoid interleaving with logs.
- Fixed initial progress bar value for `reserve` resume.
- Fixed `put` progress initialization on resume.
- Fixed timing of `PadReserved` event emission.
- Fixed `put` progress bar logic and messages.
- Fixed skipping redundant existence checks in `put_raw` for reused pads.
- Fixed `put` progress bar timing and phase display.
- Fixed `rm` to save index cache after removing key.
- Fixed `store` operation to save index cache after completion.
- Removed `delete scratchpad` functionality (likely due to issues).
- Fixed hardcoded devnet key format in CLI.
- Fixed CLI to adapt to `mutant-lib` v0.2.0 API changes.
- Fixed first testnet run issues.
- Fixed various imports in `mutant-cli` after `mutant-lib` prelude refactoring.
- Correctly handle specific `IndexError::DeserializationError` during `sync` when remote index does not exist.
- Correctly set initial status (`Allocated` or `Generated`) for pads acquired during `store` operation, fixing errors when reusing pads from `free_pads` list after a `remove`.
- Implemented pad harvesting during `remove` operation: `Generated` pads move to `pending_verification_pads`, others move to `free_pads`.
- Handle inconsistency during `put` resume where `check_existence` fails but pad actually exists on network, preventing `put_raw` from erroring on create.
- Modified resume logic (`prepare_pads_for_store`) to attempt writing `Generated` pads even if `check_existence` fails, relying on `put_raw`'s inconsistency handling.
- Increase sleep time in `manage_local_testnet.sh` to allow EVM testnet more time to start in CI.
- **CLI:** Changed `put --force` behavior to perform `remove` then `store` instead of calling the unimplemented `update` library function.

### Changed
- `mutant stat` now shows more detailed breakdown for incomplete keys.
- Refactored internal data operations (`store`, `fetch`, etc.) into separate modules.
- `KeyDetails` now includes optional `public_address`.
- `KeySummary` now indicates if an entry is public and includes its address.
- CLI `put` command now checks key status before storing/updating:
  - Errors if key exists and is complete (unless `--force` is used).
  - Resumes if key exists but is incomplete.
  - Starts new upload if key doesn't exist.
- Removed the intermediate `storage` module, directly using the `network` adapter for pad read/write operations.
- Refactored `DefaultDataManager` to remove redundant `DataManagerDependencies` struct, passing `&DefaultDataManager` directly to ops functions.
- Restricted public API surface of `mutant-lib`, re-exporting only necessary types.
- Refactored `rm` key logic: Only `Generated` pads go to pending verification; others go directly to free pool.
- Reduced default usable scratchpad size margin to 4KB.
- Refactored `put` preparation logic into `PadLifecycleManager::prepare_pads_for_store`.
- Refactored `store_op` to use a combined write/confirm loop (`execute_write_confirm_tasks`) and standalone `confirm_pad_write` function.
- Optimized `put` operation for newly generated pads by skipping unnecessary network existence checks.
- Refactored `put_raw` to use `PadStatus` to determine create/update network calls instead of create-then-update.
- Refactored `store_op` logic into preparation and execution phases.
- Moved data operations logic (`store_op`, `fetch_op`, `remove_op`) into dedicated modules (`data/ops/`).
- Changed `purge` command to be the sole mechanism for verifying `pending_verification_pads`.
- Refactored `put` operation in `mutant-lib` to perform chunk writes and network confirmations concurrently.
- Aligned `reserve` logic with `put`, fixing payment errors.
- Centralized pad reservation logic in `PadLifecycleManager`.
- Changed `purge` logic to only discard pads on `RecordNotFound`.
- **CLI:** Renamed "Reserving pads..." progress message to "Acquiring pads..." during `put`.
- **CLI:** Refactored `put` progress display to use two bars (Create/Confirm).
- **Refactor:** Removed manager traits (`NetworkAdapter`, `DataManager`, `PadLifecycleManager`, `IndexManager`, `StorageManager`) in `mutant-lib` and replaced their usage with direct calls to the corresponding struct implementations (`AutonomiNetworkAdapter`, `DefaultDataManager`, `DefaultPadLifecycleManager`, `DefaultIndexManager`, `DefaultStorageManager`). This simplifies the internal architecture by removing unnecessary abstraction layers.
- Refactored pad harvesting logic from `data::ops::remove` into a dedicated `harvest_pads` method in `IndexManager`.
- Optimized pad harvesting during `rm` operation by removing redundant network existence check.
- Made pad harvesting during `rm` fully local, moving pads to free/pending lists without network calls.
- **Refactor:** Consolidated public upload metadata into the main `MasterIndex.index` map using an `IndexEntry` enum (`PrivateKey(KeyInfo)` or `PublicUpload(PublicUploadInfo)`). Removed the separate `MasterIndex.public_uploads` field and `PublicUploadMetadata` struct.

### Removed
- Removed unused file (`src/unused.rs`? - commit `32e986ff`).
- Removed redundant pad release functions (`pad_lifecycle::pool::release_pads_to_free`, `pad_lifecycle::manager::release_pads`).
- Removed `Mutant::reserve_new_pad` function (centralized in manager).
- Removed unused `IndexManager::update_public_upload_metadata` function.

### Documentation
- Updated README.md to reflect workspace structure, latest CLI commands/options, and library API changes.
- Adjusted introductory joke in README.md for subtlety (removed italics).

## [0.2.1] - 2025-04-18

### Added
- Prompt user for confirmation before creating remote index during sync if it doesn't exist.

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
- Remove operation (`rm`) now saves the updated index to the local cache, ensuring `ls` reflects removals.
- Correctly handle errors during pad release in `IndexManager::remove_key_info_internal`.

### Changed
- Display completion percentage for incomplete keys in `ls` output (e.g., `*mykey (50.0%)`).
- Added detailed statistics section for incomplete uploads in `stats` output.
- Implemented resumable `put` operations. Uploads can now be interrupted and resumed, picking up from the last successfully written chunk.
- Introduced granular pad statuses (`Generated`, `Written`, `Confirmed`) in the index to track chunk progress accurately.
- Optimized `put` operation for newly generated pads by skipping unnecessary network existence checks before the initial write.
- Introduced `PadStatus::Allocated` to explicitly track scratchpads known to exist on the network before data write is confirmed.
- Moved chunking tests from inline module to `data::integration_tests`.
- Added integration tests for `DataManager` (`store`, `fetch`, `remove`), including checks for non-existent keys and multi-chunk data.
- Added specific integration test (`test_store_very_large_data_multi_pad`) for storing 16MiB data, verifying correct multi-pad chunking (5 pads).
- Changed `rm` key logic: Only `Generated` pads go to pending verification; `Allocated`, `Written`, and `Confirmed` pads go directly to the free pool (counters fetched automatically).

### Changed
- **Refactor (mutant-lib):** Refactored `data::ops::store::store_op` by:
    - Moving preparation logic (resume check, existence check, pad acquisition/replacement, index persistence) to `pad_lifecycle::prepare::prepare_pads_for_store`.
    - Moving confirmation logic (retry loop, counter check, index update, cache save) to an internal helper `confirm_pad_write`.
    - Moving the concurrent write/confirm execution loop to an internal helper `execute_write_confirm_tasks`.
    - `store_op` now acts primarily as an orchestrator.
- **Refactor (mutant-lib):** Moved data operation logic (`store_op`, `fetch_op`, `remove_op`) from `data/ops.rs` into a new `data/ops/` directory with separate files (`store.rs`, `fetch.rs`, `remove.rs`, `common.rs`) for better modularity.
- Refactored pad state management: `pending_verification_pads` in the index is now only used for pads associated with removed keys whose write status was uncertain (`Generated`).
- `purge` command is now the sole mechanism responsible for verifying pads in the `pending_verification_pads` list and moving them to the free pool.
- **Refactored `put` operation in `mutant-lib` to perform chunk writes and network confirmations concurrently, improving upload throughput.**
- **CLI:** Renamed "Reserving pads..." progress message during `put` to "Acquiring pads..." for clarity when reusing free pads.
- **CLI:** Refactored `put` progress display to use two bars:
  - "Creating pads...": Increments on `ChunkWritten` (network create/update success).
  - "Confirming pads...": Increments on `ChunkConfirmed` (network check success).
- Modified the `purge` command logic to only discard pending pads if the network explicitly returns a "Record Not Found" status. Pads encountering other network errors during verification are now returned to the pending list for future retries.
- Store confirmation (`

## [0.5.1] - 2024-08-06

### Fixed
- Panic in index persistence when file doesn't exist #57 @Champii
