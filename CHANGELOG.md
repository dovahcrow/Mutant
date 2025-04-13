# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Start of development for v0.1.1
- Added `reset` command to `mutant-cli` to clear the master index. Requires explicit confirmation by typing 'reset'.
- **Configuration:** Added `MutAntConfig` struct and `NetworkChoice` enum (Devnet/Mainnet) to allow configuring the MutAnt instance upon initialization. The `init` method now uses default configuration, while `init_with_progress` accepts a `MutAntConfig`.
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

### Fixed
- Ensure CLI errors are always printed to stderr in `main` in addition to being logged.
- Correctly use `--local` flag to select Devnet instead of always using Mainnet logic internally.
- When using `--local`, ensure the hardcoded devnet private key is used, overriding any selected wallet file.
- Initialize Autonomi network client (local or mainnet) based on configuration, not hardcoded to local.
- Prevent warning about `scratchpad_size` being 0 after a `reset` by ensuring the correct default size is set during the reset operation.
- Prevent double-counting of scratchpads in `stats` after a `reset` then `put` by ensuring newly reserved pads are not added to the free list and saved prematurely during the reservation step.
- Fixed pad allocation logic error occurring after `rm` then `put` by refactoring `allocate_and_write` to correctly account for freed pads.
- Ensure master index is saved correctly *after* data is written and index is updated in memory during `put` operation, preventing loss of key information on successful writes.
- Fixed compiler warnings related to unused imports and variables.
- Tolerate transient decryption errors during scratchpad creation verification loop, allowing retries instead of immediate failure.
- Prevent internal fetch loop from retrying on decryption errors, allowing the outer verification loop to handle retries with appropriate delays.
- Tolerate transient decryption errors during scratchpad update verification loop, allowing retries instead of immediate failure.

## [0.1.2] - Unreleased

### Added
- Added `--local` flag to `mutant-cli` to explicitly use the Devnet configuration.

## [0.1.1] - 2024-04-11

### Changed
- **`mutant-lib` API:** Modified `MutAnt::init` and `MutAnt::init_with_progress` to only require the `private_key_hex` string. The `autonomi::Wallet` is now derived internally.

## [0.1.0] - 2025-04-12

### Added

- Implemented the `stats` command to display detailed storage usage information (total/free/occupied pads, total/free/occupied/wasted space).
- Implemented concurrent chunk fetching (up to 20 simultaneous downloads) for the `get` command, significantly improving performance for large, chunked data.
- Reintroduced a progress bar for the `get` command to display download progress.
- Added `ls` command to list stored keys.
- Refactored scratchpad reservation in `PadManager` to be asynchronous. `reserve_new_pads` now returns immediately with a channel receiver, allowing `perform_concurrent_write` to start writing to pads as they become available instead of waiting for all reservations to complete.
- Detailed progress reporting during initialization using `InitProgressEvent::Step`.
- `ls -l` command option to display key size and last modified time.

### Changed

- Refactored `PadManager`
- `ls` command now sorts keys alphabetically by default.
- Store/update operations now record a modification timestamp.
- Adjusted `ls` short format output to only list keys, one per line.
- Refactored `PadManager::retrieve_data` into smaller helper functions for improved readability and maintainability (`get_pads_and_size`, `spawn_fetch_tasks`, `collect_fetch_results`, `update_progress`, `order_and_combine_data`).
- Refactored `mutant-lib/src/pad_manager/write.rs` into smaller modules (`reservation.rs`, `concurrent.rs`) for better readability and maintainability.
- Modified `MutAnt::init` to accept a `private_key_hex` string and derive the storage key internally via SHA256 hashing, ensuring deterministic storage addresses.
- Refactored the storage module into submodules (`init`, `network`) for better organization.
- Removed unused `ContentType::UserData` variant and `network` field from `Storage`.

### Performance

- Batch progress updates during `get`
