# Internals: Pad Management

This document details how MutAnt manages scratchpads (pads) throughout their lifecycle, from creation to reuse.

## 1. Pad Lifecycle

Each pad in MutAnt goes through a defined lifecycle, represented by the `PadStatus` enum:

```rust
pub enum PadStatus {
    Generated,  // Pad is allocated but not yet written to
    Written,    // Pad has been written to but not yet confirmed
    Confirmed,  // Pad has been successfully written and verified
    Free,       // Pad is available for reuse
    Invalid,    // Pad is invalid and cannot be used
}
```

### 1.1 Status Transitions

The typical lifecycle of a pad follows these transitions:

1. **Generated → Written**: When a pad is first written to
2. **Written → Confirmed**: When a pad write is verified
3. **Confirmed → Free**: When a key is removed
4. **Free → Generated**: When a free pad is reused
5. **Written → Invalid**: When a pad write fails verification
6. **Invalid → Free**: When an invalid pad is recycled

## 2. Pad Information

Each pad is represented by a `PadInfo` struct:

```rust
pub struct PadInfo {
    pub address: ScratchpadAddress,
    pub private_key: Vec<u8>,
    pub status: PadStatus,
    pub size: usize,
    pub checksum: u64,
    pub last_known_counter: u64,
}
```

- **address**: The Autonomi scratchpad address
- **private_key**: The encrypted private key for the pad
- **status**: The current status of the pad
- **size**: The size of the data chunk stored in the pad
- **checksum**: A checksum of the data for verification
- **last_known_counter**: The last known counter value for the pad

## 3. Pad Allocation

When a new key is created or an existing key is updated, MutAnt needs to allocate pads for storing the data chunks.

### 3.1 Allocation Process

1. Calculate the number of pads needed based on data size and storage mode
2. Check if there are enough free pads available
3. If not enough free pads, generate new ones
4. Create `PadInfo` entries for each pad with status `Generated`
5. Add the pads to the appropriate index entry

```rust
// Simplified allocation process
let mut pads = Vec::new();
let mut free_pads_used = 0;

// Try to use free pads first
while free_pads_used < needed_pads && !index.free_pads.is_empty() {
    let pad = index.free_pads.pop().unwrap();
    pads.push(PadInfo {
        address: pad.address,
        private_key: pad.private_key,
        status: PadStatus::Generated,
        size: chunk_size,
        checksum: calculate_checksum(&chunk),
        last_known_counter: pad.last_known_counter,
    });
    free_pads_used += 1;
}

// Generate new pads if needed
for _ in free_pads_used..needed_pads {
    let (address, private_key) = generate_new_pad();
    pads.push(PadInfo {
        address,
        private_key,
        status: PadStatus::Generated,
        size: chunk_size,
        checksum: calculate_checksum(&chunk),
        last_known_counter: 0,
    });
}
```

## 4. Pad Reuse

MutAnt reuses pads to avoid creating new scratchpads unnecessarily.

### 4.1 Free Pad Pool

When a key is removed, its pads are moved to the `free_pads` list in the `MasterIndex`:

```rust
// When removing a key
let entry = index.index.remove(key_name).unwrap();
let pads = match entry {
    IndexEntry::PrivateKey(pads) => pads,
    IndexEntry::PublicUpload(_, pads) => pads,
};

// Mark pads as free and add to free_pads list
for mut pad in pads {
    pad.status = PadStatus::Free;
    index.free_pads.push(pad);
}
```

### 4.2 Pad Selection

When allocating pads, MutAnt prefers to use pads from the `free_pads` list before generating new ones. This reduces the number of network operations and improves performance.

## 5. Pad Verification

MutAnt verifies pads to ensure data integrity.

### 5.1 Verification Process

1. After writing a pad, MutAnt attempts to read it back
2. The checksum of the read data is compared to the expected checksum
3. If the checksums match, the pad status is updated to `Confirmed`
4. If the checksums don't match, the pad status is updated to `Invalid`

### 5.2 Health Checks

The `health_check` operation verifies all pads for a specific key:

```rust
// Simplified health check process
for pad in pads {
    match verify_pad(&pad).await {
        Ok(_) => {
            pad.status = PadStatus::Confirmed;
        }
        Err(_) => {
            pad.status = PadStatus::Invalid;
            if recycle {
                // Move to free_pads for reuse
                index.free_pads.push(pad);
            }
        }
    }
}
```

## 6. Pad Recycling

Failed operations are recycled to ensure data integrity.

### 6.1 Recycling Mechanism

1. When a pad operation fails, the pad is sent to the recycling queue
2. The recycler task processes failed pads and attempts to recycle them
3. If recycling is successful, the pad is sent back to the global queue for processing
4. If recycling fails, the pad is marked as `Invalid`

```rust
// Simplified recycling process
match recycle_function(error_cause, item_to_recycle).await {
    Ok(Some(new_item)) => {
        // Send recycled item back to the global queue
        global_tx_for_recycler.send(new_item).await?;
    },
    Ok(None) => {
        // Item cannot be recycled, drop it
    },
    Err(recycle_err) => {
        // Recycling failed, log error
    }
}
```

## 7. Public Index Pads

For public keys, MutAnt creates a special index pad that contains metadata about the data pads.

### 7.1 Index Pad Structure

The index pad contains:
- The total size of the data
- The number of data pads
- The addresses of all data pads
- A checksum of the entire data

### 7.2 Public Address

The address of the index pad is used as the public address for the key. When updating a public key, the index pad is preserved to maintain the same public address.

## 8. Pad Status Tracking

MutAnt tracks the status of all pads to ensure data integrity and enable efficient reuse.

### 8.1 Status Updates

Pad status is updated throughout the lifecycle:
- When a pad is allocated: `Generated`
- When a pad is written to: `Written`
- When a pad is verified: `Confirmed`
- When a key is removed: `Free`

### 8.2 Status Queries

MutAnt provides methods to query pad status:
- `list()`: Returns all keys and their associated pads
- `get_storage_stats()`: Returns statistics about pad usage
- `purge()`: Verifies and cleans up invalid pads
- `health_check()`: Checks the health of a specific key's pads