# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Start of development for v0.1.1
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

### Fixed
- Ensure CLI errors are always printed to stderr in `main` in addition to being logged.
- Correctly use `--local` flag to select Devnet instead of always using Mainnet logic internally.
- Initialize Autonomi network client (local or mainnet) based on configuration, not hardcoded to local.
- When using `--local`, automatically use the hardcoded devnet/test secret key.
- Fixed pad allocation logic error occurring after `rm` then `put` by refactoring `allocate_and_write` to correctly account for freed pads.

## [0.1.1] - 2024-04-11

### Fixed
- Fixed issue where `ls -l` would panic if storage was completely empty.

## [0.1.0] - 2024-04-10

### Added
- Initial release of Mutant CLI and Library.
- Basic PUT, GET, RM, LS commands.
- Storage statistics command (`stats`).
- Progress bars for network operations.

### Changed
- **`mutant-lib` API:** Modified `MutAnt::init` and `MutAnt::init_with_progress` to only require the `private_key_hex` string. The `autonomi::Wallet` is now derived internally.

## [0.1.2] - Unreleased

### Added
- Added `--local` flag to `mutant-cli` to explicitly use the Devnet configuration.