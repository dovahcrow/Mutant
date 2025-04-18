# Public Upload Feature Implementation Plan

This document tracks the implementation of the public upload feature in `mutant-lib` and `mutant-cli`.

**Legend:**
- [ ] To Do
- [-] In Progress
- [x] Done

## `mutant-lib` Implementation

### 1. Constants and Core Structures (`mutant-lib/src`)

-   [x] **Define Data Encoding Constants:**
    -   Located `mutant-lib/src/data/mod.rs`.
    -   Defined `PUBLIC_DATA_ENCODING: u64 = 3;`
    -   Defined `PUBLIC_INDEX_ENCODING: u64 = 4;`
    -   Verified existing encodings (implicitly, assuming 1 and 2 are used correctly elsewhere).
    -   *Status: Done.*
-   [x] **Modify `MasterIndex` Struct:**
    -   In `mutant-lib/src/index/structure.rs`.
    -   Modify `index` field type: `pub index: std::collections::HashMap<String, IndexEntry>`.
    -   Add `IndexEntry` enum: `enum IndexEntry { PrivateKey(KeyInfo), PublicUpload(PublicUploadInfo) }`.
    -   Add `PublicUploadInfo` struct (fields: `address`, `size`, `modified`).

### 2. Helper Function (`mutant-lib/src/network` module)

-   [x] **Implement `create_public_scratchpad`:**
    -   *Correction:* Moved from `data/manager.rs` to the `network` module (`adapter.rs`) for better separation of concerns.
    -   Created a crate-visible free function `fn create_public_scratchpad(...) -> Scratchpad`.
    -   Added necessary imports (`Bytes`, `PublicKey`, `Scratchpad`, etc.) and cleaned up module imports.
    -   Logic implemented: Derive PublicKey, calculate ScratchpadAddress, get bytes for signature, sign bytes, call `Scratchpad::new_with_signature`.
    -   *Status: Done.*

### 3. Public Store Logic (`mutant-lib/src/data/manager.rs`)

-   [x] **Implement `DataManager::store_public`:**
    -   *Refactored to delegate to `ops::store_public::store_public_op`.*
    -   Defined `async fn store_public(...) -> Result<ScratchpadAddress, DataError>`.
    -   Calls `ops::store_public::store_public_op(self, ...).await`.
-   [ ] **Implement `ops::store_public::store_public_op` (`mutant-lib/src/data/ops/store_public.rs`):**
    -   Created file.
    -   Added `pub(crate) async fn store_public_op(data_manager: &DefaultDataManager, ...)`.
    -   Moved logic from original `DataManager::store_public`.
    -   Fixed linter errors (index access, event type, payment option, network call, error mapping).
    -   Logic Implemented:
        -   Generate new `SecretKey` (`public_sk`).
        -   Check for name collision using `get_index_copy`.
        -   Chunk data using `chunk_data` and `index_manager.get_scratchpad_size`.
        -   Iterate through chunks (sequentially):
            -   Use `network::create_public_scratchpad` helper.
            -   Upload using `network_adapter.scratchpad_put(...)` with `PaymentOption::Wallet(...)`.
            -   Collected addresses.
            -   Reported progress via callback using `PutEvent::ChunkUploadProgress`.
        -   Serialize `Vec<ScratchpadAddress>` using `serde_cbor`.
        -   Create public index scratchpad using helper.
        -   Upload index scratchpad using `scratchpad_put`.
        -   Call `index_manager.insert_public_upload_metadata(...)` to update index.
        -   Create `PublicUploadMetadata`.
        -   Insert into `public_uploads` map.
        -   Return `public_index_address`.
    -   Added TODOs for parallel uploads and payment handling.

### 4. Public Fetch Logic (`mutant-lib/src/data/manager.rs`)

-   [x] **Implement `DataManager::fetch_public`:**
    -   *Refactored to delegate to `ops::fetch_public::fetch_public_op`.*
    -   Defined `async fn fetch_public(network_adapter: &AutonomiNetworkAdapter, ...) -> Result<Bytes, DataError>`.
    -   Calls `ops::fetch_public::fetch_public_op(...)`.
-   [ ] **Implement `ops::fetch_public::fetch_public_op` (`mutant-lib/src/data/ops/fetch_public.rs`):**
    -   Created file.
    -   Added `pub(crate) async fn fetch_public_op(network_adapter: &AutonomiNetworkAdapter, ...)`.
    -   Moved logic from original `DataManager::fetch_public`.
    -   Fixed linter errors (used specific `DataError` variants).
    -   Logic Implemented:
        -   Fetch index scratchpad `network_adapter.get_raw_scratchpad(...)`.
        -   Verify signature.
        -   Verify encoding (`PUBLIC_INDEX_ENCODING`).
        -   Deserialize `scratchpad.encrypted_data()` into `Vec<ScratchpadAddress>` using `serde_cbor`.
        -   Initialize result buffer.
        -   Iterate through chunk addresses (sequentially):
            -   Fetch data scratchpad `network_adapter.get_raw_scratchpad(...)`.
            -   Verify signature.
            -   Verify encoding (`PUBLIC_DATA_ENCODING`).
            -   Append `scratchpad.encrypted_data()` to buffer.
        -   Return assembled `Bytes`.
    -   Added TODOs for parallel fetches and callback reporting.

### 5. API Layer Integration (`mutant-lib/src/api/mutant.rs`)

-   [x] **Implement `MutAnt::store_public`:**
    -   Defined `pub async fn store_public(&self, name: String, data_bytes: &[u8], callback: Option<PutCallback>) -> Result<ScratchpadAddress, Error>`.
    -   Calls `self.data_manager.store_public(...)`.
    -   Maps error to `Error::Data`.
    -   Calls `self.save_index_cache().await`. Returns `Error::Index` on failure.
    -   Returns `Ok(address)`.
-   [x] **Implement `MutAnt::get_public_address`:**
    -   Defined `pub async fn get_public_address(&self, name: &str) -> Result<Option<ScratchpadAddress>, Error>`.
    -   Calls `self.index_manager.get_index_copy().await?`.
    -   Looks up `name` in `index_copy.index`.
    -   Maps result to `Ok(Some(address))`, `Ok(None)`. Maps error to `Error::Index`.
-   [x] **Implement `MutAnt::fetch_public`:**
    -   Defined `pub async fn fetch_public(&self, public_index_address: ScratchpadAddress, ...) -> Result<Bytes, Error>` (takes `&self`).
    -   Calls static `DataManager::fetch_public(&self.network_adapter, ...)` passing the internal network adapter.
    -   Maps error to `Error::Data`.

### 6. Exports and Errors

-   [x] **Update API Exports (`mutant-lib/src/api/mod.rs`):**
    -   Added `store_public`, `get_public_address`, `fetch_public` to the `pub use` statements.
    -   *Note: Linter errors regarding duplicate exports of Callbacks persist despite multiple attempts to fix. Proceeding.*
-   [x] **Update Error Enum (`mutant-lib/src/internal_error.rs`):**
    -   Added `PublicUploadNameExists` to `IndexError` (`index/error.rs`).
    -   Added `Serialization`, `Deserialization`, `InvalidSignature`, `InvalidPublicIndexEncoding`, `InvalidPublicDataEncoding`, `PublicScratchpadNotFound` to `DataError` (`data/error.rs`).
    -   Added `From<serde_cbor::Error>` for `DataError`.
    -   Confirmed `Error` in `internal_error.rs` uses `#[from]` for `DataError` and `IndexError`, no changes needed there.

### 7. Testing (`mutant-lib/tests`)

-   [x] **Add relevant tests to modules:**
    -   Added test for `create_public_scratchpad` to `network/integration_tests.rs`.
    -   Added test for `insert_public_upload_metadata` to `index/integration_tests.rs`.
    -   Added tests for `store_public_op` (basic, empty, collision) to `data/integration_tests.rs`.
    -   Added tests for `fetch_public_op` (basic, empty, not_found, wrong_index_encoding) to `data/integration_tests.rs`.
    -   Added tests for `MutAnt` public API methods (`store_public`, `get_public_address`, `fetch_public`) to `api/mutant.rs`.
-   [ ] Write integration tests for `store_public`.
-   [ ] Write integration tests for `fetch_public` using the address from `store_public`.
-   [ ] Write integration tests for `get_public_address`.
-   [ ] Test error cases (name collision, invalid address fetch, fetching non-public data with `fetch_public`).

## `mutant-cli` Implementation

*(To be detailed after `mutant-lib` is complete)*

-   [ ] Add `store-public <NAME> <FILE>` command.
-   [ ] Add `fetch-public <ADDRESS> <OUTPUT_FILE>` command.
-   [ ] Add `get-public-address <NAME>` command.
-   [ ] Update help messages and documentation.

---

*Self-Correction/Refinement during implementation:*
*   *Initial thought*: Place `create_public_scratchpad` in `api/mutant.rs`. *Correction*: Better fit within `data/manager.rs` alongside other scratchpad interactions.
*   *Initial thought*: `fetch_public` takes `&self`. *Correction*: Made static and takes `autonomi::Client` as it doesn't need instance state, but adjusted `MutAnt::fetch_public` wrapper to pass the internal client adapter. Changed plan to reflect this.
*   *Refinement*: Clarified payment handling needs for `store_public` chunk uploads. May need `reserve_pads` logic similar to private uploads, or handle payment per-put. Requires investigation.
*   *Refinement*: Ensure `save_index_cache` is called after `store_public` in the `MutAnt` wrapper.
*   *Refinement*: Clarified error types (`DataError`, `IndexError`, `Error`).
*   *Refinement*: `DataManager::fetch_public` needs the `AutonomiNetworkAdapter`, not the raw `autonomi::Client`. Updated plan.
*   *Refinement*: `MutAnt::fetch_public` needs to pass `&self.network_adapter` not create a new client. Updated plan. 