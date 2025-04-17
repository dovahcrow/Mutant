# Internal Data Structures

These Rust structs form the core of `mutant-lib`'s state management. They are serialized using CBOR and persisted within the Master Index scratchpad.

## 1. `MasterIndexStorage`

This struct represents the entire state managed by `mutant-lib` for a given user key. It's loaded into memory at initialization and saved back to the Master Index scratchpad whenever modifications occur.

```rust
// Located likely in mutant_lib::index::types or similar
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct MasterIndexStorage {
    // Maps user-provided keys (String) to their storage metadata.
    pub(crate) index: MasterIndex, // Alias for HashMap<String, KeyStorageInfo>

    // Stores pads that are ready for reuse.
    // Each tuple contains the address and the *private key* bytes
    // needed to decrypt and overwrite the pad.
    #[serde(default)]
    pub(crate) free_pads: Vec<(ScratchpadAddress, Vec<u8>)>, 

    // The usable payload size (in bytes) for data scratchpads.
    // Determined during initialization and stored for consistency.
    // Example: PHYSICAL_SCRATCHPAD_SIZE - encryption_overhead.
    pub(crate) scratchpad_size: usize,
}
```

**Fields:**

*   `index: MasterIndex` (Type alias for `HashMap<String, KeyStorageInfo>`):
    *   The primary lookup map.
    *   Keys are the `String` identifiers provided by the user (e.g., `"my_document.txt"`).
    *   Values are `KeyStorageInfo` structs detailing how and where the data for that key is stored.
*   `free_pads: Vec<(ScratchpadAddress, Vec<u8>)>`:
    *   A list of data scratchpads that previously held data but have since been released via `MutAnt::remove`.
    *   Crucially, the `Vec<u8>` here stores the *private key* bytes for the scratchpad. This is necessary so the library can take ownership and overwrite the pad when it gets reused during a `store` operation.
    *   This list acts as a pool for efficient pad recycling.
*   `scratchpad_size: usize`:
    *   Stores the effective payload capacity of each data scratchpad, taking into account network limits and encryption overhead.
    *   This value is typically calculated once during `MutAnt::init` based on the `MutAntConfig` and persisted here to ensure that data chunking remains consistent across library sessions.

## 2. `KeyStorageInfo`

This struct holds the metadata specific to a single user key, detailing how its data is distributed across one or more data scratchpads.

```rust
// Located likely in mutant_lib::index::types or similar
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct KeyStorageInfo {
    // List of pads storing the data chunks for this key.
    // Each tuple contains the pad's address and its *public key* bytes.
    pub(crate) pads: Vec<(ScratchpadAddress, Vec<u8>)>, 

    // The total size in bytes of the original, unchunked user data.
    pub(crate) data_size: usize,

    // Timestamp of the last `store` or `update` operation for this key.
    #[serde(default = "Utc::now")] // Or appropriate default timestamp fn
    pub(crate) modified: DateTime<Utc>, 
}
```

**Fields:**

*   `pads: Vec<(ScratchpadAddress, Vec<u8>)>`:
    *   A vector where each element corresponds to one chunk of the stored data.
    *   `ScratchpadAddress`: The network address where the data chunk's scratchpad resides.
    *   `Vec<u8>`: The *public key* bytes corresponding to the scratchpad's unique encryption key. The public key is sufficient for locating the pad but not for decrypting it.
    *   The order of elements in this vector dictates the reassembly order for the data chunks during a `fetch` operation.
*   `data_size: usize`:
    *   The total size of the original data blob before it was chunked.
    *   Used during `fetch` to verify that all chunks were retrieved and reassembled correctly.
*   `modified: DateTime<Utc>`:
    *   Records the timestamp (UTC) of the last time the data for this key was successfully stored or updated.

These two structures, nested within each other and stored persistently, form the backbone of `mutant-lib`'s ability to manage arbitrary-sized data blobs using fixed-size scratchpads. 