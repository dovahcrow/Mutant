# MutAnt Library: Overview

Welcome to `mutant-lib`! This library provides a robust and convenient way to store and manage data of any size on the Autonomi network.

## The Challenge: Fixed-Size Storage

The Autonomi network uses fixed-size storage units called "scratchpads." While powerful, directly managing these scratchpads for storing large files or numerous small pieces of data can be complex. You'd need to handle:

*   Splitting large data across multiple pads.
*   Keeping track of which pads belong to which piece of data.
*   Managing encryption keys for each pad.
*   Reassembling data correctly upon retrieval.
*   Handling potential network errors and retries.
*   Reusing scratchpads efficiently when data is deleted.

## The Solution: `mutant-lib`

`mutant-lib` abstracts away these complexities, offering a simple key-value store interface built on top of Autonomi scratchpads.

**Key Features:**

*   **Arbitrary Data Size:** Store data blobs of any size, seamlessly handled behind the scenes.
*   **Simple Key-Value API:** Interact with your data using straightforward string keys (e.g., `"my_config"`, `"user_profile_123"`).
*   **Automatic Chunking & Reassembly:** Data is automatically split into chunks, stored across necessary scratchpads, and reassembled upon fetching.
*   **Metadata Management:** A central "Master Index" keeps track of where your data chunks are stored.
*   **Encryption:** All data stored in scratchpads is encrypted.
*   **Pad Reuse:** Scratchpads from deleted data are tracked and reused for efficiency.
*   **Concurrency:** Designed for safe use in asynchronous Rust applications.
*   **Progress Reporting:** Optional callbacks allow monitoring the progress of lengthy operations (like storing large files).
*   **Network Resilience:** Built-in retry logic handles transient network issues.

## How it Works (High Level)

1.  **Initialization:** You initialize `mutant-lib` with your Autonomi private key. The library finds or creates a special "Master Index" scratchpad associated with your key.
2.  **Storing Data (`store`):**
    *   You provide a key (string) and data (bytes).
    *   `mutant-lib` calculates how many scratchpads are needed.
    *   It finds reusable pads or creates new ones.
    *   It splits your data into chunks, encrypts them, and writes them to the allocated pads.
    *   It updates the Master Index with the key and the locations (addresses and public keys) of the data pads.
3.  **Fetching Data (`fetch`):**
    *   You provide the key.
    *   `mutant-lib` looks up the key in the Master Index to find the associated data pad locations.
    *   It fetches the encrypted chunks from the network.
    *   It decrypts the chunks and reassembles them into the original data.
4.  **Removing Data (`remove`):**
    *   You provide the key.
    *   `mutant-lib` removes the key's entry from the Master Index.
    *   It marks the associated data pads as "free" within the Master Index, making them available for future `store` operations.

## Next Steps

*   **Getting Started:** See `getting_started.md` for a quick tutorial on basic usage.
*   **Core Concepts:** Dive deeper into the fundamental ideas in `core_concepts.md`.
*   **Architecture:** Understand the detailed component layout in `architecture.md`.
*   **API Reference:** Explore the full public API in `api_reference.md`. 