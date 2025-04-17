# Internals: Operation Flows

This document outlines the step-by-step internal process for the core `MutAnt` operations: `store`, `fetch`, and `remove`. It shows how the API layer, Pad Manager, Storage Layer, and data structures interact.

**Legend:**

*   `MutAnt`: The public API instance.
*   `PadMgr`: The internal Pad Manager / Pad Lifecycle component.
*   `StoreLayer`: The internal Storage Layer component.
*   `IndexCache`: The `Arc<Mutex<MasterIndexStorage>>` holding the in-memory index.
*   `NetClient`: The `autonomi::Client` instance.

## 1. Store Flow (`MutAnt::store`)

**Goal:** Store `data` under `key`.

```mermaid
sequenceDiagram
    actor User
    participant MutAnt
    participant PadMgr
    participant IndexCache
    participant StoreLayer
    participant NetClient

    User->>MutAnt: store(key, data)
    MutAnt->>PadMgr: allocate_and_prepare_pads(data.len())
    PadMgr->>IndexCache: lock() and read scratchpad_size
    PadMgr->>IndexCache: lock() and check/take free_pads
    PadMgr->>PadMgr: Generate new pads if needed (derive keys/addresses)
    PadMgr->>IndexCache: unlock()
    PadMgr-->>MutAnt: Return list of (address, private_key) for writes
    MutAnt->>PadMgr: chunk_data(data, scratchpad_size)
    PadMgr-->>MutAnt: Return data_chunks
    
    MutAnt->>PadMgr: orchestrate_writes(pads_for_write, data_chunks)
    loop For each chunk/pad pair
        PadMgr->>StoreLayer: create_or_update_pad(address, private_key, chunk)
        StoreLayer->>StoreLayer: Encrypt chunk
        alt Pad was from free_pads
            StoreLayer->>NetClient: scratchpad_update(address, private_key, encrypted_chunk)
        else Pad was new
            StoreLayer->>NetClient: scratchpad_create(address, public_key, encrypted_chunk)
        end
        NetClient-->>StoreLayer: Result (with retries)
        StoreLayer-->>PadMgr: Write result
    end
    PadMgr-->>MutAnt: Overall write success/failure

    alt Writes Successful
        MutAnt->>PadMgr: prepare_key_storage_info(key, public_keys, data.len())
        PadMgr-->>MutAnt: Return KeyStorageInfo
        MutAnt->>IndexCache: lock()
        MutAnt->>IndexCache: insert(key, KeyStorageInfo)
        MutAnt->>IndexCache: unlock()
        MutAnt->>StoreLayer: save_master_index(IndexCache)
        StoreLayer->>StoreLayer: Serialize IndexCache (CBOR)
        StoreLayer->>NetClient: scratchpad_update(master_index_addr, master_index_key, serialized_index)
        NetClient-->>StoreLayer: Result
        StoreLayer-->>MutAnt: Save result
        MutAnt-->>User: Ok(())
    else Writes Failed
        MutAnt-->>User: Err(WriteError)
    end
```

**Steps:**

1.  **User Call:** `MutAnt::store(key, data)` is called.
2.  **Pad Allocation (PadMgr):**
    *   Calculates the number of pads needed.
    *   Locks `IndexCache`, checks `scratchpad_size`.
    *   Locks `IndexCache`, tries to acquire pads from `free_pads` (obtaining their private keys).
    *   Generates any additional new pads required (creating new private/public key pairs and addresses).
    *   Unlocks `IndexCache`.
    *   Returns the list of `(address, private_key)` tuples needed for writing.
3.  **Chunking (PadMgr):** Splits `data` into chunks based on `scratchpad_size`.
4.  **Concurrent Writes (PadMgr -> StoreLayer -> NetClient):**
    *   Iterates through chunks and the allocated pad list.
    *   For each pair, calls `StoreLayer::create_or_update_pad`.
    *   `StoreLayer` encrypts the chunk using the provided private key.
    *   `StoreLayer` calls `NetClient::scratchpad_create` (for new pads, using the public key) or `NetClient::scratchpad_update` (for reused pads, using the private key).
    *   Network results (including retries) propagate back.
5.  **Index Update (MutAnt -> IndexCache):**
    *   If all writes succeed, `MutAnt` asks `PadMgr` to format the `KeyStorageInfo` (using the *public* keys of the pads).
    *   `MutAnt` locks `IndexCache` and inserts the new `(key, KeyStorageInfo)` mapping.
    *   `MutAnt` unlocks `IndexCache`.
6.  **Persist Index (MutAnt -> StoreLayer -> NetClient):**
    *   `MutAnt` calls `StoreLayer::save_master_index`.
    *   `StoreLayer` serializes the current `IndexCache` content to CBOR.
    *   `StoreLayer` calls `NetClient::scratchpad_update` to save the serialized index to the Master Index pad.
7.  **Return:** `MutAnt` returns `Ok(())` or an appropriate `Error` to the user.

## 2. Fetch Flow (`MutAnt::fetch`)

**Goal:** Retrieve the data associated with `key`.

```mermaid
sequenceDiagram
    actor User
    participant MutAnt
    participant PadMgr
    participant IndexCache
    participant StoreLayer
    participant NetClient

    User->>MutAnt: fetch(key)
    MutAnt->>IndexCache: lock() and read KeyStorageInfo for key
    alt Key Found
        IndexCache-->>MutAnt: Return KeyStorageInfo { pads: Vec<(addr, pubkey)>, data_size }
        MutAnt->>IndexCache: unlock()
        MutAnt->>PadMgr: derive_private_keys(pads)
        PadMgr->>PadMgr: For each pad, Derive(UserPrivateKey, addr)
        PadMgr-->>MutAnt: Return Vec<(addr, private_key)>
        
        MutAnt->>PadMgr: orchestrate_reads(pads_with_private_keys)
        loop For each (address, private_key) pair
            PadMgr->>StoreLayer: fetch_pad(address, private_key)
            StoreLayer->>NetClient: scratchpad_get(address, private_key)
            NetClient-->>StoreLayer: Result (encrypted_chunk)
            StoreLayer->>StoreLayer: Decrypt chunk (spawn_blocking)
            StoreLayer-->>PadMgr: Decrypted chunk result
        end
        PadMgr->>PadMgr: Reassemble chunks in order
        PadMgr->>PadMgr: Verify total size == data_size from KeyStorageInfo
        alt Size OK
             PadMgr-->>MutAnt: Return assembled Vec<u8>
             MutAnt-->>User: Ok(Vec<u8>)
        else Size Mismatch
             PadMgr-->>MutAnt: Err(InternalError)
             MutAnt-->>User: Err(InternalError)
        end
    else Key Not Found
        IndexCache-->>MutAnt: KeyNotFound
        MutAnt->>IndexCache: unlock()
        MutAnt-->>User: Err(KeyNotFound)
    end
```

**Steps:**

1.  **User Call:** `MutAnt::fetch(key)` is called.
2.  **Index Lookup (MutAnt -> IndexCache):**
    *   `MutAnt` locks `IndexCache`.
    *   Looks up `key` in the `index` map.
    *   If found, retrieves the `KeyStorageInfo` (containing the list of `(address, public_key)` tuples and `data_size`).
    *   If not found, unlocks and returns `Error::KeyNotFound`.
    *   Unlocks `IndexCache`.
3.  **Derive Private Keys (MutAnt -> PadMgr):**
    *   `MutAnt` passes the list of `(address, public_key)` tuples to `PadMgr`.
    *   `PadMgr` iterates through the list, deterministically deriving the *private key* for each `address` using the main user private key.
    *   Returns the list of `(address, private_key)` tuples.
4.  **Concurrent Reads (PadMgr -> StoreLayer -> NetClient):**
    *   `PadMgr` iterates through the `(address, private_key)` list.
    *   For each pair, calls `StoreLayer::fetch_pad`.
    *   `StoreLayer` calls `NetClient::scratchpad_get` using the address and private key.
    *   `StoreLayer` receives the encrypted chunk.
    *   `StoreLayer` decrypts the chunk (potentially offloading to `spawn_blocking`).
    *   Decrypted chunks are returned to `PadMgr`.
5.  **Reassembly & Verification (PadMgr):**
    *   `PadMgr` collects all decrypted chunks.
    *   Reassembles them in the original order (based on the `KeyStorageInfo.pads` list).
    *   Compares the size of the reassembled data with `KeyStorageInfo.data_size`.
6.  **Return:**
    *   If size matches, `PadMgr` returns the `Vec<u8>` to `MutAnt`, which returns `Ok(data)` to the user.
    *   If size mismatches, returns `Error::InternalError`.

## 3. Remove Flow (`MutAnt::remove`)

**Goal:** Remove `key` and recycle its pads.

```mermaid
sequenceDiagram
    actor User
    participant MutAnt
    participant PadMgr
    participant IndexCache
    participant StoreLayer
    participant NetClient

    User->>MutAnt: remove(key)
    MutAnt->>IndexCache: lock()
    MutAnt->>IndexCache: remove(key) -> Option<KeyStorageInfo>
    alt Key Found and Removed
        IndexCache-->>MutAnt: Return removed KeyStorageInfo { pads: Vec<(addr, pubkey)> }
        MutAnt->>PadMgr: harvest_pads(pads)
        PadMgr->>PadMgr: For each pad, Derive(UserPrivateKey, addr) -> private_key
        PadMgr-->>MutAnt: Return Vec<(addr, private_key)>
        MutAnt->>IndexCache: append pads_with_private_keys to free_pads list
        MutAnt->>IndexCache: unlock()

        MutAnt->>StoreLayer: save_master_index(IndexCache)
        StoreLayer->>StoreLayer: Serialize IndexCache (CBOR)
        StoreLayer->>NetClient: scratchpad_update(master_index_addr, master_index_key, serialized_index)
        NetClient-->>StoreLayer: Result
        StoreLayer-->>MutAnt: Save result
        MutAnt-->>User: Ok(())
    else Key Not Found
        IndexCache-->>MutAnt: None (key wasn't present)
        MutAnt->>IndexCache: unlock()
        MutAnt-->>User: Ok(()) // Removing non-existent key is often OK
    end
```

**Steps:**

1.  **User Call:** `MutAnt::remove(key)` is called.
2.  **Index Update (MutAnt -> IndexCache):**
    *   `MutAnt` locks `IndexCache`.
    *   Calls `remove(key)` on the `index` map. This returns the `KeyStorageInfo` if the key existed.
3.  **Pad Harvesting (MutAnt -> PadMgr):**
    *   If a `KeyStorageInfo` was returned (key existed):
        *   `MutAnt` passes the `pads` list (`Vec<(address, public_key)>`) to `PadMgr`.
        *   `PadMgr` derives the *private key* for each address.
        *   `PadMgr` returns the list of `(address, private_key)` tuples.
        *   `MutAnt` appends this list to the `free_pads` vector within the locked `IndexCache`.
4.  **Unlock Index:** `MutAnt` unlocks `IndexCache`.
5.  **Persist Index (MutAnt -> StoreLayer -> NetClient):**
    *   If the key existed and pads were harvested (or even if the key didn't exist, saving is often harmless), `MutAnt` calls `StoreLayer::save_master_index`.
    *   `StoreLayer` serializes the potentially updated `IndexCache` content to CBOR.
    *   `StoreLayer` calls `NetClient::scratchpad_update` to save the index.
6.  **Return:** `MutAnt` returns `Ok(())` to the user (typically, removing a non-existent key is not an error). 