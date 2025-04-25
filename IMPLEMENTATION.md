# WebSocket Progress Events Implementation Plan

## Overview

This document outlines the plan to implement progress event forwarding from the daemon to the client through WebSocket, maintaining the same progress bar behavior as the original CLI implementation.

## Components to Modify

1. Protocol (mutant-protocol)
2. Daemon (mutant-daemon)
3. Client (mutant-client)
4. CLI (mutant-cli)

## Implementation Steps

### 1. Protocol Enhancement

- [ ] Add new progress event types in `mutant-protocol/src/lib.rs`:
  ```rust
  #[derive(Serialize, Deserialize, Debug, Clone)]
  pub enum TaskProgress {
      Put(PutProgressEvent),
      // ... other progress types
  }

  #[derive(Serialize, Deserialize, Debug, Clone)]
  pub enum PutProgressEvent {
      Starting {
          total_chunks: usize,
          initial_written_count: usize,
          initial_confirmed_count: usize,
          chunks_to_reserve: usize,
      },
      PadReserved,
      PadsWritten,
      PadsConfirmed,
      Complete,
  }
  ```

### 2. Daemon Enhancement

- [ ] Modify `mutant-daemon/src/handler.rs` to forward lib events:
  ```rust
  impl Handler {
      async fn handle_put(&self, request: PutRequest, ws_tx: UnboundedSender<Response>) -> Result<TaskId, Error> {
          let task_id = Uuid::new_v4();
          
          let callback: PutCallback = Arc::new(move |event| {
              let tx = ws_tx.clone();
              let task_id = task_id;
              Box::pin(async move {
                  let progress = match event {
                      PutEvent::Starting { .. } => TaskProgress::Put(PutProgressEvent::Starting { .. }),
                      PutEvent::PadReserved => TaskProgress::Put(PutProgressEvent::PadReserved),
                      PutEvent::PadsWritten => TaskProgress::Put(PutProgressEvent::PadsWritten),
                      PutEvent::PadsConfirmed => TaskProgress::Put(PutProgressEvent::PadsConfirmed),
                      PutEvent::Complete => TaskProgress::Put(PutProgressEvent::Complete),
                  };

                  let _ = tx.send(Response::TaskUpdate(TaskUpdateResponse {
                      task_id,
                      status: TaskStatus::InProgress,
                      progress: Some(progress),
                  }));

                  Ok(true)
              })
          });

          self.mutant.set_put_callback(callback).await;
          // ... rest of put handling
      }
  }
  ```

### 3. Client Enhancement

- [ ] Add blocking put operation in `mutant-client/src/lib.rs`:
  ```rust
  impl MutantClient {
      pub async fn put_blocking(&mut self, user_key: &str, data: &[u8]) -> Result<(), ClientError> {
          let (complete_tx, complete_rx) = oneshot::channel();
          let (event_tx, event_rx) = mpsc::channel(100);
          
          // First clone the client for the event handling task
          let mut client_clone = self.clone();
          
          // Spawn the event listening task BEFORE making the put request
          let event_handle = tokio::spawn(async move {
              while let Some(response) = client_clone.next_response().await {
                  match response {
                      Ok(Response::TaskUpdate(update)) => {
                          if let Some(TaskProgress::Put(event)) = update.progress {
                              let _ = event_tx.send(event).await;
                              
                              if matches!(event, PutProgressEvent::Complete) {
                                  break;
                              }
                          }
                      }
                      Ok(Response::TaskResult(result)) => {
                          match result.status {
                              TaskStatus::Completed => {
                                  let _ = complete_tx.send(Ok(()));
                                  break;
                              }
                              TaskStatus::Failed => {
                                  let err = result.result.and_then(|r| r.error)
                                      .unwrap_or_else(|| "Unknown error".to_string());
                                  let _ = complete_tx.send(Err(ClientError::TaskFailed(err)));
                                  break;
                              }
                              _ => {}
                          }
                      }
                      Err(e) => {
                          let _ = complete_tx.send(Err(e));
                          break;
                      }
                      _ => {}
                  }
              }
          });

          // Now make the put request
          let task_id = self.put(user_key, data).await?;

          Ok((event_rx, complete_rx, event_handle))
      }
  }
  ```

### 4. CLI Enhancement

- [ ] Update put command in `mutant-cli/src/main.rs` to use the new blocking put:
  ```rust
  async fn handle_put(client: &mut MutantClient, key: &str, file: &str) -> Result<()> {
      let data = std::fs::read(file)?;
      
      let multi_progress = MultiProgress::new();
      let (res_pb_opt, upload_pb_opt, confirm_pb_opt, _) = create_put_callback(&multi_progress, false);
      
      // Start the put operation which gives us our event channels and task handle
      let (mut event_rx, complete_rx, event_handle) = client.put_blocking(key, &data).await?;

      // Handle progress updates
      while let Some(event) = event_rx.recv().await {
          match event {
              PutProgressEvent::Starting { 
                  total_chunks,
                  initial_written_count,
                  initial_confirmed_count,
                  chunks_to_reserve,
              } => {
                  let mut res_pb_guard = res_pb_opt.lock().await;
                  if chunks_to_reserve > 0 {
                      let res_pb = res_pb_guard.get_or_insert_with(|| {
                          let pb = StyledProgressBar::new_for_steps(&multi_progress);
                          pb.set_message("Acquiring pads...".to_string());
                          pb
                      });
                      res_pb.set_length(chunks_to_reserve as u64);
                      res_pb.set_position(0);
                  }
                  // ... rest of Starting event handling from put.rs
              },
              PutProgressEvent::PadReserved => {
                  let mut res_pb_guard = res_pb_opt.lock().await;
                  if let Some(pb) = res_pb_guard.as_mut() {
                      if !pb.is_finished() {
                          pb.inc(1);
                      }
                  }
              },
              // ... handle other events exactly as in put.rs
              PutProgressEvent::Complete => {
                  // Clear progress bars as in put.rs
                  let mut res_pb_guard = res_pb_opt.lock().await;
                  if let Some(pb) = res_pb_guard.take() {
                      pb.finish_and_clear();
                  }
                  // ... clear other progress bars
              }
          }
      }

      // Wait for both the event handling and completion
      let result = complete_rx.await?;
      event_handle.await?;
      
      result
  }
  ```

## Progress

### Protocol
- [x] Add TaskProgress enum
- [x] Add PutProgressEvent enum
- [x] Update TaskUpdateResponse to include progress (was already compatible)

### Daemon
- [ ] Add event forwarding in handle_put
- [ ] Test event forwarding

### Client
- [ ] Add put_blocking method
- [ ] Add event handling
- [ ] Test blocking put operation

### CLI
- [ ] Update put command
- [ ] Add progress bar handling
- [ ] Test progress display

## Testing Plan

1. Unit Tests
   - [ ] Test protocol serialization/deserialization
   - [ ] Test client event handling
   - [ ] Test daemon event forwarding

2. Integration Tests
   - [ ] Test full put flow with progress
   - [ ] Test error handling
   - [ ] Test cancellation

## Current Status

Not started.

## Next Steps

1. Start with protocol enhancement
2. Then daemon modifications
3. Then client implementation
4. Finally CLI updates 