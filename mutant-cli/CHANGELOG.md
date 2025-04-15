## [Unreleased]

### Added
- Global `--quiet` (`-q`) flag to suppress progress bars and other non-essential output.

### Changed
- Parallelized pad verification during `purge` command for faster execution.

### Fixed
- **Compatibility:** Updated CLI commands (`get`, `put`, `ls`, `reset`, `purge`, `sync`, `import`, `rm`) and callbacks to align with `mutant-lib` v0.2.0 API changes (event structures, error handling, method signatures, `Send + Sync` requirements).
- **Sync Logic:** Re-added `get_index_copy` to `mutant-lib` API to support existing CLI sync merge strategy.
- **Ls Command:** Corrected type handling in `ls` short format to properly display key status.
- **Reset Command:** Fixed `reset` command to call the correct `mutant.reset()` method.
- Corrected checksum logic in `put` command to prevent unnecessary overwrites.

## [0.1.1] - 2024-05-21

### Fixed
- Resolved issue where pad verification could hang indefinitely.
- Improved error message clarity for network timeouts during verification.

## [0.1.0] - 2024-05-20

Initial Release 