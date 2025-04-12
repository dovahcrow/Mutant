# MutAnt Library: Overview

This document provides a high-level overview of the `mutant-lib`, a Rust library designed for storing and managing arbitrarily sized data blobs on the Autonomi network, abstracting away the fixed-size limitations of individual scratchpads.

## 1. Core Problem

The Autonomi network provides storage through fixed-size units called "scratchpads". Storing data larger than a single scratchpad or managing numerous small pieces of data efficiently requires a higher-level abstraction. `mutant-lib` provides this abstraction layer.

## 2. Key Concepts

*   **User Keys:** Data blobs are stored and retrieved using simple string keys provided by the user (e.g., "my_document.txt", "config_v1").
*   **Master Index:** A central, persistent data structure stored in its own dedicated scratchpad. It maps user keys to metadata about how and where the corresponding data is stored across one or more data scratchpads. It also tracks free (unused) scratchpads.
*   **Data Scratchpads:** Standard Autonomi scratchpads used to store the actual bytes of user data. A single user key's data might span multiple data scratchpads if it exceeds the usable size of one pad.
*   **Pad Manager:** An internal component responsible for allocating scratchpads for new data, finding free pads, splitting data across pads, reading data that spans pads, and managing the list of free pads in the Master Index.
*   **Storage Layer:** The lowest-level internal component that directly interacts with the Autonomi network client (`autonomi::Client`). It handles raw scratchpad operations (create, get, update), data serialization/deserialization (using CBOR), and encryption/decryption.

## 3. High-Level Architecture

```
+---------------------+      +---------------------+      +------------------------+
|     Application     |----->|    MutAnt API       |----->|      Pad Manager       |
| (Using mutant-lib)  |      | (mutant::MutAnt)    |      | (pad_manager::PadManager)|
+---------------------+      +---------------------+      +------------------------+
                                     |                            |
                                     |                            | Uses/Updates
                                     v                            v
+---------------------+      +---------------------+      +-------------------------+
|   Storage Layer     |<-----| Master Index Cache  |      |   Master Index Storage  |
| (storage::Storage)  |      | (Arc<Mutex<MIS>>)   |----->| (Persistent Scratchpad) |
+---------------------+      +---------------------+      +-------------------------+
          |
          | Interacts via autonomi::Client
          v
+---------------------+      +---------------------+      +---------------------+
| Data Scratchpad 1   |      | Data Scratchpad 2   | ...  | Data Scratchpad N   |
| (Autonomi Network)  |      | (Autonomi Network)  |      | (Autonomi Network)  |
+---------------------+      +---------------------+      +---------------------+
```

1.  **Application:** Interacts with the public `MutAnt` API (`store`, `fetch`, `remove`, etc.).
2.  **`MutAnt` API:** The main entry point. Holds references to the `PadManager` and `Storage` layer. Manages access to the in-memory `MasterIndexStorage` cache via a thread-safe `Arc<Mutex<MasterIndexStorage>>`. It delegates complex storage logic to the `PadManager`.
3.  **Pad Manager:** Receives requests from `MutAnt`. Consults/updates the `MasterIndexStorage` cache to find storage locations or free pads. Determines how data should be split or combined across pads. Instructs the `Storage` layer to perform necessary reads/writes/creates/updates on specific data scratchpads.
4.  **Master Index Cache:** An in-memory, thread-safe copy of the `MasterIndexStorage`. This is read and modified by the `PadManager` and occasionally by `MutAnt` directly (e.g., for stats).
5.  **Storage Layer:** Executes low-level commands from the `PadManager` (e.g., "fetch bytes from pad X", "update pad Y with these bytes"). Interacts directly with the `autonomi::Client`. It is also responsible for loading the `MasterIndexStorage` from its dedicated scratchpad on initialization and saving it back when changes occur.
6.  **Master Index Storage:** The single source of truth for metadata, persisted in a dedicated scratchpad on the Autonomi network. Contains the map of user keys to data locations and the list of free pads.
7.  **Data Scratchpads:** Where the actual user data bytes are stored, encrypted using unique keys. Managed by the `PadManager` via the `Storage` layer.

## 4. Basic Usage Flow (Conceptual)

*   **Initialization (`MutAnt::init`):**
    *   The `Storage` layer is initialized, connecting to the Autonomi network.
    *   It attempts to load the `MasterIndexStorage` from the designated scratchpad address. If not found, it creates a new, empty one.
    *   The `PadManager` is initialized with references to the `Storage` layer and the `MasterIndexStorage` cache.
    *   A `MutAnt` instance is returned.
*   **Store (`MutAnt::store`):**
    *   `MutAnt` passes the request to the `PadManager`.
    *   `PadManager` checks the `MasterIndexStorage` cache for available free pads or determines how many new pads are needed.
    *   It instructs the `Storage` layer to create/update the necessary data scratchpads with chunks of the user's data.
    *   `PadManager` updates the `MasterIndexStorage` cache with the new key and its associated pad locations/keys.
    *   `MutAnt` triggers the `Storage` layer to save the updated `MasterIndexStorage` back to its persistent scratchpad.
*   **Fetch (`MutAnt::fetch`):**
    *   `MutAnt` passes the request to the `PadManager`.
    *   `PadManager` looks up the key in the `MasterIndexStorage` cache to find the list of data scratchpads and keys.
    *   It instructs the `Storage` layer to fetch the data from each required scratchpad.
    *   `PadManager` reassembles the data chunks and returns the complete byte vector.
*   **Remove (`MutAnt::remove`):**
    *   `MutAnt` passes the request to the `PadManager`.
    *   `PadManager` looks up the key in the `MasterIndexStorage` cache.
    *   It removes the key entry and adds the associated data scratchpads (address and key) to the `free_pads` list in the cache.
    *   `MutAnt` triggers the `Storage` layer to save the updated `MasterIndexStorage`. (Note: Data pads are not immediately overwritten, just marked as free for reuse).

## 5. Key Features

*   **Arbitrary Data Size:** Stores data larger than a single scratchpad.
*   **Abstraction:** Hides the complexity of scratchpad management from the user.
*   **Persistence:** Metadata (Master Index) is stored persistently on the network.
*   **Encryption:** Data in each scratchpad is encrypted.
*   **Concurrency:** Uses `Arc<Mutex<>>` for safe concurrent access to the Master Index.
*   **Progress Reporting:** Supports callbacks for long-running operations (`init`, `store`, `fetch`).
*   **Retry Logic:** Incorporates retries for potentially transient network errors during storage operations.

See `internals.md` for a deeper dive into data structures and component logic, and `usage.md` for practical API examples. 