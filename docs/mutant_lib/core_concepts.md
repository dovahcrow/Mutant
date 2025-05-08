# MutAnt Library: Core Concepts

Understanding these core concepts is key to effectively using and reasoning about `mutant-lib`.

## 1. Master Index

* **What it is:** The single source of truth for your data stored via `mutant-lib`. It's a data structure (`MasterIndex`) containing metadata about all the keys you've stored.
* **Where it lives:** It's stored within its *own* dedicated Autonomi scratchpad, separate from the scratchpads holding your actual data chunks.
* **Identifier:** The address and encryption key for this Master Index scratchpad are derived *deterministically* from the user's primary private key provided during `MutAnt::init`. This ensures that only the owner of the private key can access their index.
* **Content:**
  * A map from user-provided `String` keys to `IndexEntry` enum variants.
  * A list of "free" scratchpads (`PadInfo` structs) that were previously used for data but are now available for reuse (populated when `rm` is called).
  * A list of "pending verification" scratchpads that need to be verified.
  * The network choice (Mainnet, Devnet, Alphanet).
* **Persistence:** The library loads this index into memory (`Arc<RwLock<MasterIndex>>`) during initialization. Any operation that modifies the index (e.g., `put`, `rm`) triggers a save back to its dedicated scratchpad on the network.
* **Analogy:** Think of it like the index card catalog in a library. It tells you which shelf (data scratchpad address) and which specific book identifier (public key) corresponds to the title (user key) you're looking for. It also has a list of empty shelf slots (free pads).

## 2. Data Scratchpads

* **What they are:** Standard Autonomi scratchpads used to store the actual bytes of your data.
* **Size Limit:** Each scratchpad has a physical size limit. The storage mode (`Small`, `Medium`, `Large`) determines the chunk size used for storing data.
* **Allocation:**
  * When you `put` data, the library calculates how many pads are needed based on the storage mode.
  * It first tries to reuse pads from the `free_pads` list in the Master Index.
  * If more pads are needed, it generates new ones, each with its own unique address and encryption key pair.
* **Encryption:** The data chunk stored in *each* data scratchpad is encrypted using a unique key associated with that specific pad.
* **Status Tracking:** Each pad has a status (`Generated`, `Written`, `Confirmed`, `Free`, `Invalid`) that tracks its lifecycle.

## 3. Index Entries

* **What they are:** Entries in the Master Index, associated with each user key.
* **Types:**
  * `PrivateKey(Vec<PadInfo>)`: For private keys, contains a list of pad information.
  * `PublicUpload(PadInfo, Vec<PadInfo>)`: For public keys, contains a special index pad and a list of data pads.
* **Pad Info (`PadInfo`):**
  * `address`: The Autonomi scratchpad address.
  * `private_key`: The encrypted private key for the pad.
  * `status`: The current status of the pad.
  * `size`: The size of the data chunk stored in the pad.
  * `checksum`: A checksum of the data for verification.
  * `last_known_counter`: The last known counter value for the pad.
* **Purpose:** Provides the necessary information to locate all the chunks comprising the data associated with a user key.

## 4. Public/Private Keys

* **Private Keys:** By default, data is stored privately and can only be accessed by the owner of the private key.
* **Public Keys:** Data can be stored publicly, making it accessible to anyone who knows the public address.
* **Index Pad:** Public keys have a special index pad that contains metadata about the data pads.
* **Public Address:** The address of the index pad is used as the public address for the key.
* **Preservation:** When updating a public key, the index pad is preserved to maintain the same public address.

## 5. Worker Architecture

* **Worker Pool:** Operations are processed by a pool of workers.
* **Concurrency:** Each worker manages a batch of concurrent tasks (default: 10 tasks per worker).
* **Clients:** Each worker has exactly ONE dedicated client to prevent blocking.
* **Distribution:** Tasks are distributed to workers in round-robin fashion initially.
* **Work Stealing:** Workers can steal work from the global queue when their local queue is empty.
* **Recycling:** Failed operations are sent to a recycling queue for retry.
* **Completion:** Completion is determined by counting confirmed pads rather than checking if queues are empty.

## 6. Pad Lifecycle

* **Generated:** Pad is allocated but not yet written to.
* **Written:** Pad has been written to but not yet confirmed.
* **Confirmed:** Pad has been successfully written and verified.
* **Free:** Pad is available for reuse.
* **Invalid:** Pad is invalid and cannot be used.

## 7. Pad Reuse

* When `MutAnt::rm(key)` is called:
  * The entry for `key` is removed from the `index` map in the `MasterIndex`.
  * The associated pads are moved to the `free_pads` list in the `MasterIndex`.
* When `MutAnt::put(key, data)` is called:
  * The library checks the `free_pads` list first.
  * If suitable pads are found, they are removed from `free_pads` and used for storing the new data chunks.
  * Only if `free_pads` is empty or insufficient are new pads generated.
* **Benefit:** Reduces the need to constantly create new scratchpads on the network, potentially saving costs and time.

## 8. Storage Modes

* **Small:** Uses smaller chunks (128KB), suitable for small files or when minimizing per-chunk overhead is important.
* **Medium:** Uses medium-sized chunks (2MB), a good balance for most use cases.
* **Large:** Uses larger chunks (8MB), suitable for large files when minimizing the number of network operations is important.

## 9. Callbacks and Progress Reporting

* **Callbacks:** Operations like `put`, `get`, `purge`, and `health_check` accept optional callback functions.
* **Events:** Callbacks receive events like `PadReserved`, `PadWritten`, `PadConfirmed`, etc.
* **Progress Tracking:** Callbacks allow monitoring the progress of lengthy operations.
* **Cancellation:** Some callbacks allow cancelling operations in progress.