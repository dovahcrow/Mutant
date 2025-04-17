# MutAnt Library: Best Practices & Tips

Here are some recommendations for using `mutant-lib` effectively and efficiently.

## 1. Handling Large Data

*   **Use Progress Callbacks:** For `store` and `fetch` operations involving large amounts of data (spanning many scratchpads), use the `_with_progress` variants (`store_with_progress`, `fetch_with_progress`). This allows your application to provide feedback to the user about the ongoing operation and potentially offer cancellation.
*   **Consider Streaming (If Available):** If the library or application context supports streaming reads/writes, this could be more memory-efficient than loading the entire `Vec<u8>` into memory for very large blobs. (Check if `mutant-lib` offers streaming APIs; if not, this might be a future enhancement suggestion).
*   **Appropriate `scratchpad_size`:** While often handled by default, ensure the configured `scratchpad_size` (in `MutAntConfig`) is reasonable. Too small increases the number of pads and network requests; too large might exceed network limits or increase the impact of single pad failures.

## 2. Key Management

*   **Choose Meaningful Keys:** Use descriptive keys (`String`) that make sense for your application domain (e.g., `"user:123:profile_image"`, `"config:prod:v3"`).
*   **Avoid Very Long Keys:** While technically possible, extremely long keys might have minor storage/performance implications in the index.
*   **Namespace Keys:** Consider using prefixes or delimiters (like `:` or `/`) in your keys to create logical namespaces, making `list_keys` more manageable if you store many different types of data.

## 3. Error Handling

*   **Handle `KeyNotFound` Gracefully:** Expect that `fetch` or `remove` might target keys that don't exist. Use `matches!` or `if let` to specifically handle the `Error::KeyNotFound` variant.
*   **Retry Logic Awareness:** Understand that the library performs internal retries for network operations. Errors propagated to your application (like `Error::AutonomiClient` or `Error::TimeoutError`) usually indicate a more persistent failure after retries have been exhausted.
*   **Log Errors:** Log unexpected errors (like `Error::InternalError`, `Error::SerializationError`, etc.) to help with debugging.

## 4. Performance and Efficiency

*   **Reuse `MutAnt` Instance:** Initialization (`MutAnt::init`) involves network communication to load the Master Index. Avoid initializing a new `MutAnt` instance for every operation; create one instance (potentially shared using `Arc`) and reuse it.
*   **Understand `remove` vs. Purge:** `remove` only marks pads as reusable in the index; it doesn't delete data immediately. This is usually efficient. If you need to guarantee data erasure (which might not be fully possible on a distributed network anyway), different strategies would be needed (and might not be offered by the library).
*   **Consider `reserve_pads`:** If you know you'll be storing a batch of new data soon, calling `reserve_pads` beforehand *might* slightly speed up the subsequent `store` calls by pre-allocating the necessary scratchpads, though the benefit depends on network conditions and usage patterns.

## 5. Concurrency

*   **Share `MutAnt` with `Arc`:** If you need to use the `MutAnt` instance from multiple concurrent tasks or threads, wrap it in an `Arc`: `let shared_mutant = Arc::new(mutant);`.
*   **Be Mindful of Index Lock:** While data operations run concurrently, operations modifying the index (store, remove, reserve) acquire a mutex. Avoid holding references across `.await` points that might cause contention if not handled carefully with the mutex guard's lifetime.

## 6. Configuration (`MutAntConfig`)

*   Review the fields available in `MutAntConfig` (e.g., retry attempts, timeouts, custom network endpoints if applicable) and adjust them from the defaults if necessary for your specific environment or requirements. 