# Scratchpad Memory Allocator Design

## 1. Overview

This document outlines the design for a memory allocator that manages storage across multiple scratchpads, treating them as a contiguous, extensible block of memory. The goal is to provide a unified way to store data of varying sizes without size-specific logic, similar to how `malloc` manages system memory.

## 2. Core Concepts

*   **Virtual Address Space:** We'll conceptualize the scratchpads as forming a single, large virtual address space. Addresses will be represented by a combination of `(scratchpad_id, offset)`.
*   **Blocks:** The memory space is divided into blocks. Each block has a starting address, a length, and a status (Used or Free).
*   **Extensibility:** When no suitable free block exists for a new allocation request, the allocator will request new scratchpads, effectively extending the virtual address space.

## 3. Data Structures

We need efficient ways to track both used and free blocks.

### 3.1. Block Representation

A `Block` struct can represent a segment of memory:

```rust
struct Block {
    start_scratchpad_id: u64, // ID of the scratchpad where the block starts
    start_offset: usize,      // Byte offset within the start scratchpad
    length: usize,            // Total length of the block in bytes
    status: BlockStatus,      // Free or Used
    data_id: Option<DataId>,  // Identifier for the data stored (if Used)
}

enum BlockStatus {
    Free,
    Used,
}

// Assuming a unique ID type for stored data items
type DataId = u128;
```

### 3.2. Memory Map

A primary data structure will maintain the state of all memory. A simple approach is a sorted list or vector of `Block` structs, ordered by their starting address.

```rust
struct MemoryMap {
    blocks: Vec<Block>,
    total_scratchpads: usize,
    scratchpad_size: usize, // Assuming fixed size for simplicity, could be dynamic
}
```

Ordering allows for efficient merging of free blocks and finding insertion points.

### 3.3. Free List (Optimization)

For faster allocation, especially with many blocks, maintaining a separate data structure indexing *only* the free blocks could be beneficial. Options include:

*   **Segregated Free Lists:** Multiple lists, each holding free blocks within a certain size range. This speeds up finding blocks of a specific size.
*   **Tree-based Structure:** A balanced binary search tree (like a B-Tree or Red-Black Tree) storing free blocks, ordered by size or address, allowing logarithmic time complexity for searches, insertions, and deletions.

*Initial Implementation:* Start with the simple `Vec<Block>` in `MemoryMap` and consider optimizing with a dedicated free list later if performance becomes an issue.

## 4. Algorithms

### 4.1. Allocation (`allocate(size: usize) -> Result<DataId, AllocationError>`)

1.  **Search for Free Block:** Iterate through the `MemoryMap` (or the dedicated free list) to find a suitable free block.
    *   **Strategy:**
        *   **First-Fit:** Use the first free block that is large enough. Simple and fast.
        *   **Best-Fit:** Use the smallest free block that is large enough. Reduces fragmentation but can be slower and leave tiny, unusable fragments.
        *   *(Chosen Strategy):* Start with **First-Fit** for simplicity.
2.  **Block Found:**
    *   If the found free block is exactly the requested size: Change its status to `Used`, assign a new `DataId`, and return it.
    *   If the found free block is larger:
        *   Split the block into two:
            *   A `Used` block of the requested `size`. Assign a `DataId`.
            *   A `Free` block with the remaining size.
        *   Update the `MemoryMap` (replace the original free block with the new used and free blocks).
        *   Return the `DataId` of the used block.
3.  **No Suitable Block Found:**
    *   Calculate how many new scratchpads are needed to accommodate the `size`.
    *   Request new scratchpads from the system/environment. Update `total_scratchpads`.
    *   Append a new `Used` block representing the newly allocated space at the end of the virtual address space. Assign a `DataId`.
    *   If the allocation created more space than immediately needed (e.g., allocated a full scratchpad for a smaller request), add a corresponding `Free` block for the remainder.
    *   Update the `MemoryMap`.
    *   Return the `DataId`.

### 4.2. Deallocation (`free(data_id: DataId) -> Result<(), DeallocationError>`)

1.  **Find Block:** Locate the `Used` block in the `MemoryMap` corresponding to the `data_id`.
2.  **Mark as Free:** Change the block's status to `Free` and remove the `data_id`.
3.  **Merge Free Blocks:**
    *   Check the *previous* block in the `MemoryMap`. If it exists and is `Free`, merge the current block into it (update the previous block's length). Remove the current block entry.
    *   Check the *next* block in the `MemoryMap`. If it exists and is `Free`, merge it into the current (or already merged previous) block (update length). Remove the next block entry.
    *   Merging helps combat fragmentation.

### 4.3. Handling Data Spanning Scratchpads

The `Block` structure inherently handles this. A block's `length` can exceed the `scratchpad_size`. When reading or writing data associated with a `DataId`:

1.  Find the corresponding `Block`.
2.  Use `start_scratchpad_id`, `start_offset`, and `length` to determine the sequence of scratchpads and byte ranges involved.
3.  Perform read/write operations across these boundaries.

## 5. API Definition (Conceptual)

A potential `MemoryAllocator` trait or struct:

```rust
trait MemoryAllocator {
    // Initialize with initial scratchpad count and size
    fn new(initial_scratchpads: usize, scratchpad_size: usize) -> Self;

    // Allocate memory, returns an ID to reference the data
    fn allocate(&mut self, size: usize) -> Result<DataId, AllocationError>;

    // Free previously allocated memory
    fn free(&mut self, data_id: DataId) -> Result<(), DeallocationError>;

    // Get location and length of data
    fn locate(&self, data_id: DataId) -> Option<(u64, usize, usize)>; // (start_scratchpad, start_offset, length)

    // Higher-level functions might interact with scratchpad storage directly
    // fn read(&self, data_id: DataId) -> Result<Vec<u8>, ReadError>;
    // fn write(&mut self, data_id: DataId, data: &[u8]) -> Result<(), WriteError>; // Might involve reallocation if size changes
}
```

## 6. Future Considerations

*   **Performance Optimization:** Implement more sophisticated free list management (segregated lists, trees) if the simple vector approach proves too slow.
*   **Reallocation:** Add a `reallocate(data_id: DataId, new_size: usize)` function.
*   **Error Handling:** Define specific `AllocationError`, `DeallocationError`, etc.
*   **Concurrency:** If multiple threads access the allocator, add necessary locking mechanisms.
*   **Variable Scratchpad Sizes:** Adapt the logic if scratchpads can have different sizes. 