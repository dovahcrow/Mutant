# Internals: Pad Manager / Pad Lifecycle

*(Note: The specific module might be named `pad_lifecycle`, `data::ops`, or similar in the source code. This document describes the conceptual "Pad Manager" responsibilities.)*

The Pad Manager is the central coordinator for using data scratchpads. It doesn't interact directly with the network (that's the Storage Layer's job) but instead makes decisions about *which* pads to use, *how* to use them, and *what* data goes into them. It heavily relies on accessing and modifying the `MasterIndexStorage` cache.

## Key Responsibilities

1.  **Pad Allocation:** Determining which scratchpads (addresses and keys) will be used for a `store` operation.
2.  **Data Chunking:** Splitting user data into appropriately sized chunks for individual pads.
3.  **Pad Recycling:** Utilizing pads from the `free_pads` list whenever possible.
4.  **Pad Creation:** Requesting the generation of new pads when the free list is insufficient.
5.  **Operation Orchestration:** Instructing the Storage Layer to perform the necessary `create`, `update`, or `get` operations on specific pads.
6.  **Index Updates:** Preparing the `KeyStorageInfo` to be inserted into the `MasterIndexStorage` after a successful `store`.
7.  **Pad Harvesting:** Adding pads to the `free_pads` list during a `remove` operation.

## Pad Allocation Logic (for `store`)

When `store(key, data)` is called, the Pad Manager executes the following steps:

1.  **Calculate Needs:** Determine the number of pads required (`num_pads_needed`) based on `data.len()` and the `scratchpad_size` retrieved from the locked `MasterIndexStorage`.
2.  **Acquire Index Lock:** Obtain a mutable lock on the `Arc<Mutex<MasterIndexStorage>>`.
3.  **Check Free Pads:** Examine the `free_pads` list.
    *   Take up to `num_pads_needed` pads from the end of the `free_pads` list. Remember, these come with their *private keys*.
    *   Keep track of how many pads were successfully obtained from the free list (`num_from_free`).
4.  **Determine New Pad Count:** Calculate the number of *new* pads still required: `num_new_pads = num_pads_needed - num_from_free`.
5.  **Generate New Pads (if `num_new_pads > 0`):
    *   For each new pad needed:
        *   Generate a new, random `SecretKey` (private key).
        *   Derive the corresponding `PublicKey`.
        *   Derive the `ScratchpadAddress` from the public key.
        *   Store the `(ScratchpadAddress, private_key_bytes)` tuple for the upcoming write operation.
        *   Store the `(ScratchpadAddress, public_key_bytes)` tuple for inclusion in the `KeyStorageInfo` later.
6.  **Prepare Pad List:** Assemble the final list of pads to be used for writing. This list will contain tuples of `(ScratchpadAddress, private_key_bytes)`, combining pads taken from `free_pads` and newly generated ones.
7.  **Release Index Lock:** The lock on `MasterIndexStorage` can often be released here, *before* the network I/O begins, minimizing contention.

## Data Chunking & Writing (for `store`)

After allocating pads:

1.  **Chunk Data:** Split the input `data` bytes into chunks, ensuring each chunk's size is less than or equal to the `scratchpad_size`.
2.  **Concurrent Writes:** Iterate through the chunks and the prepared list of `(ScratchpadAddress, private_key_bytes)`.
    *   For each chunk/pad pair:
        *   Determine if the pad was originally from the `free_pads` list or newly generated.
        *   Instruct the `Storage Layer` to perform either:
            *   `storage.update_scratchpad(address, private_key, chunk_data)` (if reusing a free pad).
            *   `storage.create_scratchpad(address, private_key, chunk_data)` (if using a newly generated pad).
    *   These storage operations are typically spawned as concurrent `tokio` tasks.
    *   Use `futures::future::join_all` (or similar) to await the completion of all writes.
    *   Report progress via the `ProgressCallback` if provided.
3.  **Handle Results:** Check the results of all write operations. If any fail (even after retries handled by the Storage Layer), the overall `store` operation should fail.

## Index Update (for `store`)

If all writes succeed:

1.  **Acquire Index Lock:** Obtain the lock on `MasterIndexStorage` again.
2.  **Create `KeyStorageInfo`:** Construct a new `KeyStorageInfo` instance containing:
    *   The list of `(ScratchpadAddress, public_key_bytes)` for *all* pads used (both reused and new).
    *   The original `data.len()` as `data_size`.
    *   The current timestamp (`Utc::now()`) as `modified`.
3.  **Update Index Map:** Insert the new `KeyStorageInfo` into the `index` `HashMap` within the `MasterIndexStorage`, using the user's `key`.
4.  **Implicit Save:** Release the lock. The calling function (e.g., `MutAnt::store`) is responsible for ensuring `Storage::save_master_index` is called afterwards to persist the changes.

## Pad Harvesting (for `remove`)

When `remove(key)` is called:

1.  **Acquire Index Lock:** Obtain a lock on `MasterIndexStorage`.
2.  **Lookup and Remove:** Find the entry for `key` in the `index` map. If found, remove it and retrieve the `KeyStorageInfo`.
3.  **Derive Private Keys:** If the key was found, iterate through the `pads` list (`Vec<(ScratchpadAddress, public_key_bytes)>`) in the removed `KeyStorageInfo`.
    *   For each `(address, _)` tuple, derive its corresponding *private key* using the deterministic derivation function (`Derive(UserPrivateKey, address)`).
4.  **Add to Free List:** Append the derived `(address, private_key_bytes)` tuples to the `free_pads` list within the locked `MasterIndexStorage`.
5.  **Implicit Save:** Release the lock. The `MutAnt::remove` function ensures the updated `MasterIndexStorage` is saved.

This careful management of pad allocation, reuse, and harvesting is crucial for the performance and efficiency of `mutant-lib`. 