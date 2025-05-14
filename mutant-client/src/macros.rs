#[macro_export]
macro_rules! direct_request {
    ($self:expr, $key:ident, $($req:tt)*) => {{
        let key = PendingRequestKey::$key;
        let req = Request::$key(mutant_protocol::$($req)*);

        // Safely check if a request is already pending
        let is_pending = match $self.pending_requests.lock() {
            Ok(guard) => guard.contains_key(&key),
            Err(e) => {
                error!("Failed to lock pending_requests mutex: {:?}", e);
                return Err(ClientError::InternalError(
                    format!("Failed to lock pending_requests mutex: {:?}", e)
                ));
            }
        };

        if is_pending {
            return Err(ClientError::InternalError(
                format!("Another {} request is already pending", stringify!($key))
            ));
        }

        // Create the channel and pending sender
        let (sender, receiver) = oneshot::channel();
        let pending_sender = PendingSender::$key(sender);

        // Safely insert the pending request
        if let Err(e) = $self.pending_requests.lock().map(|mut guard| {
            guard.insert(key.clone(), pending_sender);
        }) {
            error!("Failed to lock pending_requests mutex for insert: {:?}", e);
            return Err(ClientError::InternalError(
                format!("Failed to lock pending_requests mutex for insert: {:?}", e)
            ));
        }

        // Send the request and handle the response
        match $self.send_request(req).await {
            Ok(_) => {
                debug!("{} request sent, waiting for response...", stringify!($key));

                // Wait for the response
                match receiver.await {
                    Ok(result) => {
                        debug!("{} response received", stringify!($key));
                        result
                    },
                    Err(e) => {
                        // Clean up on error
                        if let Ok(mut guard) = $self.pending_requests.lock() {
                            guard.remove(&key);
                        }
                        error!("{} response channel canceled: {:?}", stringify!($key), e);
                        Err(ClientError::InternalError(
                            format!("{} response channel canceled", stringify!($key))
                        ))
                    }
                }
            }
            Err(e) => {
                // Clean up on error
                if let Ok(mut guard) = $self.pending_requests.lock() {
                    guard.remove(&key);
                }
                error!("Failed to send {} request: {:?}", stringify!($key), e);
                Err(e)
            }
        }
    }};
}

#[macro_export]
macro_rules! long_request {
    // Version without streaming
    ($self:expr, $key:ident, $($req:tt)*) => {{
        let key = PendingRequestKey::TaskCreation;
        let req = Request::$key(mutant_protocol::$($req)*);
        if $self.pending_requests.lock().unwrap().contains_key(&key) {
            return Err(ClientError::InternalError(
                "Another put/get request is already pending".to_string(),
            ));
        }

        let (completion_tx, completion_rx) = oneshot::channel();
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();

        let (task_creation_tx, task_creation_rx) = oneshot::channel();
        $self.pending_requests.lock().unwrap().insert(
            key.clone(),
            PendingSender::TaskCreation(
                task_creation_tx,
                (completion_tx, progress_tx, None),
                TaskType::$key,
            ),
        );

        let start_task = async move {
            match $self.send_request(req).await {
                Ok(_) => {
                    debug!("{} request sent, waiting for TaskCreated response...", stringify!($key));
                    let task_id = task_creation_rx.await.map_err(|_| {
                        ClientError::InternalError("TaskCreated channel canceled".to_string())
                    })??;

                    info!("Task created with ID: {}", task_id);

                    completion_rx.await.map_err(|_| {
                        error!("Completion channel canceled");
                        ClientError::InternalError("Completion channel canceled".to_string())
                    })?
                }
                Err(e) => {
                    error!("Failed to send {} request: {:?}", stringify!($key), e);
                    $self.pending_requests.lock().unwrap().remove(&key);
                    Err(e)
                }
            }
        };

        Ok((start_task, progress_rx, None))
    }};

    // Version with streaming support
    ($self:expr, $key:ident, $($req:tt)*, $stream_data:expr) => {{
        let key = PendingRequestKey::TaskCreation;
        let req = Request::$key(mutant_protocol::$($req)*);
        if $self.pending_requests.lock().unwrap().contains_key(&key) {
            return Err(ClientError::InternalError(
                "Another put/get request is already pending".to_string(),
            ));
        }

        let (completion_tx, completion_rx) = oneshot::channel();
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();

        // Create data stream channel if streaming is enabled
        let (data_stream_tx, data_stream_rx) = if $stream_data {
            let (tx, rx) = mpsc::unbounded_channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let (task_creation_tx, task_creation_rx) = oneshot::channel();
        $self.pending_requests.lock().unwrap().insert(
            key.clone(),
            PendingSender::TaskCreation(
                task_creation_tx,
                (completion_tx, progress_tx, data_stream_tx),
                TaskType::$key,
            ),
        );

        let start_task = async move {
            match $self.send_request(req).await {
                Ok(_) => {
                    debug!("{} request sent, waiting for TaskCreated response...", stringify!($key));
                    let task_id = task_creation_rx.await.map_err(|_| {
                        ClientError::InternalError("TaskCreated channel canceled".to_string())
                    })??;

                    info!("Task created with ID: {}", task_id);

                    completion_rx.await.map_err(|_| {
                        error!("Completion channel canceled");
                        ClientError::InternalError("Completion channel canceled".to_string())
                    })?
                }
                Err(e) => {
                    error!("Failed to send {} request: {:?}", stringify!($key), e);
                    $self.pending_requests.lock().unwrap().remove(&key);
                    Err(e)
                }
            }
        };

        Ok((start_task, progress_rx, data_stream_rx))
    }};
}
