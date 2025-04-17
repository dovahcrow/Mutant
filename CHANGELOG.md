# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Added basic integration tests for `PadLifecycleManager::reserve_pads` covering success and failure scenarios.
- Added integration tests for `PadLifecycleManager::purge` covering existing, non-existent, mixed, and empty pending lists.

### Fixed
- **Fix:** Corrected imports and type mismatches in index integration tests (`mutant-lib/src/index/integration_tests.rs`) to allow them to compile.
- Enhanced store confirmation logic to verify decrypted data size matches expected size, improving robustness against stale reads after pad reuse.

### Changed
- Reduced default usable scratchpad size (`DEFAULT_SCRATCHPAD_SIZE`) to leave a 4KB margin instead of 512B, as an experiment to mitigate potential issues with nearly full pads on mainnet.
- Display completion percentage for incomplete keys in `ls` output (e.g., `*mykey (50.0%)`).
- Added detailed statistics section for incomplete uploads in `stats` output.
- Refactored `put` preparation logic into `PadLifecycleManager::prepare_pads_for_store` to handle resume, new uploads, and pad replacement cleanly.
- Refactored `store_op` to use a combined write/confirm loop managed by `execute_write_confirm_tasks`.
- Refactored `confirm_pad_write` to be a standalone function.
- Optimized `put` operation for newly generated pads by skipping unnecessary network existence checks before the initial write.
- Introduced `PadStatus::Allocated` to explicitly track scratchpads known to exist on the network before data write is confirmed.

### Removed
- Removed redundant pad release functions (`pad_lifecycle::pool::release_pads_to_free`, `pad_lifecycle::manager::release_pads`) as the logic is now handled within `IndexManager` and `purge`.

### Testing
- Moved chunking tests from inline module to `data::integration_tests`.
- Added integration tests for `DataManager` (`store`, `fetch`, `remove`), including checks for non-existent keys and multi-chunk data.

### Workarounds
- **Workaround:** Avoid calling `scratchpad_update` in `put_raw` due to suspected SDK bug causing data truncation on retrieval. Assume existing scratchpad is correct if `create` fails with "already exists". This may lead to stale data if the intent was to overwrite.
- Set `is_new_hint` correctly in `put` preparation based on pad origin (`Generated` vs `FreePool`) to avoid incorrect network calls.

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
- Store confirmation (`put`) now verifies by fetching and attempting decryption, not just checking existence.
- Confirmation for updated pads (from free pool) now verifies that the scratchpad counter has incremented, retrying several times with delays to allow for network propagation.
- Replaced `