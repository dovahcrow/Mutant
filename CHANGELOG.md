# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - YYYY-MM-DD

### Added
- Placeholder for 0.3.0 changes.

## [Unreleased]

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

### Removed
- Removed unused file (`src/unused.rs`? - commit `32e986ff`).
- Removed redundant pad release functions (`pad_lifecycle::pool::release_pads_to_free`, `pad_lifecycle::manager::release_pads`).
- Removed `Mutant::reserve_new_pad` function (centralized in manager).

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