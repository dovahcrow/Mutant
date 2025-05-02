# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- **Daemon:** Modified Put/Get operations to use local file paths (`source_path`, `destination_path`) on the daemon instead of transferring file data over WebSocket. The daemon now reads from/writes to its local filesystem directly.
- Refactored mutant-client to support parallel task execution:
  - Added task-specific channels for progress and completion events
  - Improved progress tracking and error handling
  - Better separation of concerns between task management and WebSocket communication
  - Simplified client API with clear completion and progress handling
  - Enhanced CLI to use the new parallel task execution features
- Optimized task query to be a simple request/response without channels
- Made the `get` method in `mutant-client` behave like the `put` method, returning a `Future` and `ProgressReceiver`.
- Updated `mutant-cli` to handle the new return type of the `client.get` method.
- Task cancellation support: Implemented `StopTask` request handling in the daemon and client to allow aborting ongoing tasks.
- Refactored pad processing pipeline to use MPMC channel (`async-channel`), removing mutex bottleneck and enabling concurrent pad dequeuing by workers.
- Refactored pad processing worker logic to achieve true concurrency by spawning tasks for each pad and managing semaphore permits correctly.
- Refactor PUT/GET operations to use generic `ops::worker::WorkerPool`.
- Refactored the `purge` operation to use the worker pool for concurrent pad processing.

### Fixed
- Fix task query WebSocket handling to prevent premature connection closure
- Fix task query display in CLI to properly show progress events and task results
- Fix task creation in mutant-client to properly initialize tasks in the ClientTaskMap when receiving TaskCreatedResponse, enabling proper progress tracking
- Fix write pipeline deadlock: Ensure main pipeline tasks (`put_private_pads`, `confirm_private_pads`) don't hold sender handles needed by each other's receiver loops by passing clones into the tasks and removing explicit drops within them.
- Fix process hang by ensuring concurrent write/confirm tasks terminate using MPSC channels.
- Adapt to `scratchpad_put` SDK changes where it no longer returns a `Receipt`. Code now correctly destructures the `(AttoTokens, ScratchpadAddress)` tuple and avoids updating the `PadInfo.receipt` field, resolving compilation errors.
- Correctly fetch and store network counter for harvested pads added to the free list, ensuring accurate `initial_counter` for `PadOrigin::FreePool`.
- Initialize `PadInfo.last_known_counter` using the `initial_counter` from `PadOrigin` when preparing pads, ensuring accurate initial counter tracking for pads from the free pool.
- Increase WebSocket maximum message size limit to 2GB in mutant-daemon to handle large data transfers.
- Fix `mutant-cli ls` command by correctly tagging `Request` enum variants in `mutant-protocol`, preventing misinterpretation of `ListKeysRequest` as `ListTasksRequest` by the daemon.
- Daemon now correctly handles its lock file, ensuring it's released upon termination.
- Fix linter errors in `pad_processing_worker_semaphore` related to `select!` type inference and `process_single_pad_task` return type after concurrency refactor.
- Fix borrow checker errors in spawned task logging and error handling within `process_single_pad_task` after concurrency refactor.
- Corrected `Network::put` and `Network::get` calls in integration tests to match updated signatures requiring an explicit client.
- Reduced peak memory usage during large PUT operations by eliminating chunk data duplication. Data is now read into an Arc<Vec<u8>> and chunk ranges are calculated. Network operations process data by slicing the original Arc based on these ranges.
- Resolved worker concurrency issue in `put` operation by removing unnecessary Mutex around client, allowing each worker to process tasks up to its `batch_size` concurrently.
- Fixed double counting of confirmed pads during `put` operation, preventing incorrect final count mismatch errors.
- Prevent `resume` operation from proceeding if the number of pads in the index mismatches the number required for the current data size, forcing a rewrite instead.
- Fix resumed `put` operations incorrectly reporting missing chunks by accounting for previously confirmed chunks in the final check.
- Propagate errors from pad recycling during `put` operation to prevent incorrect 'incomplete' errors when recycling fails.
- Return error immediately if the pad recycler task panics during `put` operation, preventing potential incorrect 'incomplete' errors.

### Changed
- Refactored write pipeline: Replaced two-stage put/confirm tasks with a single processing loop (`process_pads`) using `tokio::select!` and `FuturesUnordered` to manage concurrent `process_pad_task` operations (put -> confirm cycle) for each pad, improving deadlock resilience.
- Refactored pad preparation logic to use a single `match` statement for determining initial status and counter based on `PadOrigin`.
- Optimized resume preparation: Removed upfront concurrent `check_existence` calls for `Generated` pads. `put_raw` now attempts creation and falls back to update if the pad already exists.
- Refactored `mutant-lib::data::Data::put` method to extract private pad putting and confirmation logic into helper functions.
- Modified pad confirmation retry logic to use a total time budget instead of fixed retries and sleep.
- Refactored mutant-cli to use the new mutant-client library
- Implemented basic task list functionality in mutant-cli
- Improved mutant-cli subcommands structure:
  - Simplified `put` command syntax: `mutant put <key> <file>`
  - Added `tasks` subcommand with `list` and `get` operations
  - Better organization of task-related operations
- Removed unused tokio-tungstenite and futures-util dependencies from mutant-client
- Enhanced logging in mutant-client for better visibility of operations and task progress
- Added a new health check endpoint to verify the connection to the network.

### Added
- Added integration test (`test_generated_pad_counter_increment`) to verify scratchpad counter increments correctly on successive writes to generated pads.
- Initial implementation of `mutant-daemon`:
    - WebSocket server on `/ws`.
    - JSON-based request/response protocol.
    - Background task management system (UUID based).
    - Handles `Put` and `Get` requests (private, Medium mode only).
    - Handles `QueryTask` and `ListTasks` requests.
    - Basic error handling and reporting via WebSocket.
- Added put command to mutant-cli to store data from files
- Added wallet management system to mutant-daemon:
  - Configuration file support in XDG config directory
  - Command-line private key override option
  - Proper network choice handling (Mainnet/Devnet/Alphanet)
  - Persistent wallet configuration
  - Support for multiple wallets (one per network)
  - Automatic ANT wallet scanning and selection
  - Interactive wallet selection when multiple wallets are found
- Added background task in MutantClient to process WebSocket responses continuously

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
- Aligned `reserve`