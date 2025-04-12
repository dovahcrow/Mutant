# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Start of development for v0.1.1
- **Documentation:**
    - Replaced outdated `docs/memory_allocator_design.md` with a new set of detailed documentation for `mutant-lib`:
        - `docs/mutant_lib/overview.md`: High-level architecture and concepts.
        - `docs/mutant_lib/internals.md`: Detailed explanation of data structures, component logic (PadManager, Storage), error handling, and concurrency.
        - `docs/mutant_lib/usage.md`: Practical API usage examples.

### Changed
- Renamed `anthill-cli` to `mutant-cli` internally and directory structure.
- **Documentation:** Simplified usage example in `docs/mutant_lib/usage.md` by removing callbacks and progress reporting details.
- **Documentation:** Removed all `println!` calls from the `run_mutant_examples` function in `docs/mutant_lib/usage.md`.

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