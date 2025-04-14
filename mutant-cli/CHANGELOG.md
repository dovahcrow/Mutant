## [Unreleased]

### Added
- Global `--quiet` (`-q`) flag to suppress progress bars and other non-essential output.

### Changed
- Parallelized pad verification during `purge` command for faster execution.

### Fixed
- Corrected checksum logic in `put` command to prevent unnecessary overwrites.

## [0.1.1] - 2024-05-21

### Fixed
- Resolved issue where pad verification could hang indefinitely.
- Improved error message clarity for network timeouts during verification.

## [0.1.0] - 2024-05-20

Initial Release 