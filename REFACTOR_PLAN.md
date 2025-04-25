# Mutant Client Cross-Platform Refactor Plan

**Goal:** Refactor the `mutant-client` library to support both native (CLI) and WebAssembly (WASM) environments using a single codebase for WebSocket communication.

**Chosen Crate:** `ewebsock`

**Status:**
*   [ ] Not Started
*   [x] In Progress
*   [ ] Completed
*   [ ] Blocked

---

## Detailed Steps

### 1. Dependency Management ✅
*   [x] Add `ewebsock` dependency to `mutant-client/Cargo.toml`. (Done)
*   [x] Add necessary native dependencies (e.g., `tokio` for async runtime and task spawning) under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`. (Done)
*   [x] Review existing dependencies, remove unnecessary WASM-specific ones if `ewebsock` handles them (e.g., parts of `web-sys` might become redundant). Keep `wasm-bindgen`, `wasm-bindgen-futures`, `js-sys` as they are likely still needed for WASM specifics beyond WebSockets. (Removed WebSocket features from `web-sys`)

### 2. Refactor `MutantClient` Struct ✅
*   [x] Remove WASM-specific WebSocket fields: `ws: Option<Ws>`, `_onopen_callback`, `_onmessage_callback`, `_onerror_callback`, `_onclose_callback`.
*   [x] Add `ewebsock` related fields:
    *   `sender: Option<ewebsock::WsSender>`
    *   Receiver handled in task, not stored in struct
*   [x] Update state management: Replace `Rc<RefCell<T>>` with `Arc<Mutex<T>>` for shared state (`tasks`, `pending_task_creation`, `pending_task_list`) to make it thread-safe for native targets.

### 3. Refactor `connect` Method ✅
*   [x] Replace `WebSocket::new()` with `ewebsock::connect()`.
*   [x] Remove WASM callback setup logic (`Closure::wrap`, `set_on...`).
*   [x] Store the returned `WsSender` and `WsReceiver` in the `MutantClient` struct fields.
*   [x] Handle connection errors returned by `ewebsock::connect()`.
*   [x] **Initiate the message receiving loop/task** after successful connection.

### 4. Implement Message Receiving Loop ✅
*   [x] **Native Implementation (`#[cfg(not(target_arch = "wasm32"))]`):**
    *   Spawn a `tokio` task that owns or has access to the `WsReceiver` and the shared client state (`Arc<Mutex<...>>`).
    *   The task should loop, calling `receiver.try_recv()` or an async equivalent.
    *   On receiving a message, parse it (`ewebsock::WsEvent::Message`) and call `process_response`.
    *   Handle `ewebsock::WsEvent::Opened`, `Closed`, `Error` appropriately within the loop.
*   [x] **WASM Implementation (`#[cfg(target_arch = "wasm32")]`):**
    *   Use `wasm-bindgen-futures::spawn_local` to create an async task.
    *   This task needs access to the `WsReceiver` and client state.
    *   Loop and await/poll the receiver.
    *   Call `process_response` on `WsEvent::Message`.
    *   Handle other `WsEvent` types.
*   [x] Ensure the receiver task/loop is properly managed (e.g., canceled/stopped when the client disconnects or is dropped).

### 5. Refactor `send_request` Method ✅
*   [x] Modify signature if `&self` is sufficient (likely is).
*   [x] Use the stored `sender: Option<ewebsock::WsSender>` to send messages.
    *   Serialize the `Request` to JSON string.
    *   Send using `sender.send(ewebsock::WsMessage::Text(json_string))`.
*   [x] Remove `ws.ready_state()` checks; rely on `ewebsock` sender state or error handling.

### 6. Refactor `process_response` Method ✅
*   [x] Change signature: `fn process_response(response: Response, tasks: &ClientTaskMap, pending_task_creation: &PendingTaskCreationSender, pending_task_list: &PendingTaskListSender)`.
*   [x] Handle different `ewebsock::WsEvent` types:
    *   `WsEvent::Message(message)`:
        *   Get text content (`WsMessage::Text(text)`).
        *   Deserialize `text` into `Response`.
        *   Use existing `match response { ... }` logic.
        *   Access `tasks`, `pending_task_creation`, `pending_task_list` directly (behind `Mutex` lock).
    *   `WsEvent::Opened`: Log connection established.
    *   `WsEvent::Error(e)`: Log error, signal errors to pending futures.
    *   `WsEvent::Closed`: Log closure, signal errors/closure to pending futures.
*   [x] Adapt access to shared state (`tasks`, pending channels) using `Mutex::lock()`.

### 7. Refactor Public API Methods (`put`, `get`, `list_tasks`, `query_task`) ✅
*   [x] Update `&mut self` vs `&self` based on whether they need to modify internal state like pending requests.
*   [x] Keep the `oneshot::channel()` mechanism for requests expecting specific responses (`TaskCreated`, `TaskList`).
*   [x] Ensure `send_request` is called correctly.
*   [x] The `receiver.await` logic still works, as `process_response` (running in the background task) resolves the `oneshot::Sender`.
*   [x] Ensure error handling clears the pending `oneshot::Sender` correctly.

### 8. Concurrency Abstraction ✅
*   [x] Identified need for `Arc<Mutex<T>>` for shared state (`tasks`, `pending_...`). (Covered in Step 2)
*   [x] Implement the change from `Rc<RefCell<T>>` to `Arc<Mutex<T>>`.
*   [x] Ensure locking/unlocking is done correctly in `process_response` and any other place accessing this shared state.

### 9. Error Handling ✅
*   [x] Adapt `ClientError` enum:
    *   Add/modify variants for `ewebsock` specific errors.
    *   Update errors related to connection state (`NotConnected`, `WebSocketError`).
*   [x] Ensure errors from `ewebsock::connect`, `sender.send`, and `receiver.try_recv`/`WsEvent::Error` are propagated or handled appropriately.

### 10. Conditional Compilation (`#[cfg(...)]`) ✅
*   [x] Identified need for `cfg` for message loop task spawning. (Covered in Step 4)
*   [x] Apply `#[cfg(target_arch = "wasm32")]` and `#[cfg(not(target_arch = "wasm32"))]` where implementation differs.
*   [x] Ensure common code remains platform-agnostic.

### 11. Build & Test
*   [ ] Set up basic native main/test function to connect and interact.
*   [ ] Build for native target (`cargo build`).
*   [ ] Build for WASM target (`wasm-pack build` or similar).
*   [ ] Run tests for both environments if possible.

---

## Self-Correction/Notes during refactor:

1. Initially tried to keep `Rc<RefCell<>>` for WASM, but realized `Arc<Mutex<>>` works fine for both targets and is cleaner.
2. Removed unnecessary callback setup as `ewebsock` handles the platform differences internally.
3. Added proper error propagation and state management with `ConnectionState` enum.
4. Fixed several type mismatches with `Option<Sender>` handling in pending tasks.
5. Ensured proper error string cloning to avoid moved value issues.
6. Removed unnecessary `ready_state` checks as `ewebsock` handles connection state internally. 