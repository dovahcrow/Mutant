# TODO

## Core

- [ ] Save reserved scratchpads as soon as they are reserved to avoid loosing them
- [ ] Upload/Download/Reserve queue (set max concurrent operations)
- [ ] Concurrent reserve + upload (double progress bar, start upload on available scratchpad and upload on reserved)
- [ ] Upload/Download/Reserve retry logic
- [ ] Continue mode
- [ ] Rename entry
- [ ] Config/settings
- [ ] Wallet import from ant

## UX

- [ ] Reservation confirmation on disappering one-liner
- [ ] Reservation question shows stats
- [ ] Remove stats from ls command
- [ ] Better standalone stat command
- [ ] Confirmation on delete
- [ ] Verbose mode (-v INFO, -vv DEBUG, -vvv TRACE) + hide progress bars (and -q mode)
- [ ] Sort keys in ls command
- [ ] Hierarchical key display in ls command (with ':' or '/' or something else)
- [ ] Add a spinner for the first run vault write at the end

## R&D

- [ ] Research better storage technics than MasterIndexStorage and contiguous block allocation
- [ ] FS overlay
- [ ] Mountable storage driver
- [ ] Compression ?
- [ ] Github CI/CD