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
- Store operation confirmation was insufficient, potentially marking data as confirmed even if it was corrupted or undecryptable, or if an update didn't actually increment the counter.
- Confirmation could fail prematurely if network propagation delayed the visibility of the updated scratchpad counter.

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
- Remove operation (`rm`) now saves the updated index to the local cache, ensuring `ls`