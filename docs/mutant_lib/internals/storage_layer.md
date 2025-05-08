# Internals: Storage Layer

This document details how MutAnt interacts with the Autonomi network to store and retrieve data.

## 1. Network Module

The `Network` module is responsible for all interactions with the Autonomi network. It provides a high-level interface for the rest of the library to use without needing to understand the details of the Autonomi API.

```rust
pub struct Network {
    client_manager: Arc<Mutex<ClientManager>>,
    network_choice: NetworkChoice,
}
```

### 1.1 Key Components

- **ClientManager**: Manages Autonomi client instances
- **NetworkChoice**: Specifies which network to connect to (Mainnet, Devnet, Alphanet)

## 2. Client Management

MutAnt uses a client-per-worker approach to prevent operations from blocking each other.

### 2.1 Client Creation

```rust
pub fn get_client(&self) -> Result<Arc<Mutex<Object<ClientManager>>>, Error> {
    let client_manager = self.client_manager.lock().await;
    let client = client_manager.get_client()?;
    Ok(client)
}
```

### 2.2 Client Usage

Each worker in the worker pool has exactly ONE dedicated client:

```rust
// In worker creation
let client = network.get_client()?;
let worker = Worker::new(
    worker_id,
    client,
    task.clone(),
    worker_rx,
    global_rx.clone(),
    retry_tx.clone(),
    context.clone(),
);
```

## 3. Scratchpad Operations

The Network module provides methods for interacting with scratchpads on the Autonomi network.

### 3.1 Put Operation

```rust
pub async fn put(
    &self,
    pad_info: &PadInfo,
    data: &[u8],
    encoding: Encoding,
    is_public: bool,
) -> Result<PutResult, Error> {
    let client = self.get_client().await?;
    let client = client.lock().await;

    // Create or update the scratchpad
    let result = match pad_info.status {
        PadStatus::Generated | PadStatus::Free => {
            // Create a new scratchpad
            client.create_scratchpad(
                &pad_info.address,
                &pad_info.private_key,
                data,
                encoding,
                is_public,
            ).await?
        },
        _ => {
            // Update an existing scratchpad
            client.update_scratchpad(
                &pad_info.address,
                &pad_info.private_key,
                data,
                encoding,
                is_public,
            ).await?
        }
    };

    Ok(result)
}
```

### 3.2 Get Operation

```rust
pub async fn get(
    &self,
    address: &ScratchpadAddress,
    private_key: &[u8],
) -> Result<GetResult, Error> {
    let client = self.get_client().await?;
    let client = client.lock().await;

    // Get the scratchpad data
    let result = client.get_scratchpad(address, private_key).await?;

    Ok(result)
}
```

### 3.3 Public Get Operation

```rust
pub async fn get_public(
    &self,
    address: &ScratchpadAddress,
) -> Result<GetResult, Error> {
    let client = self.get_client().await?;
    let client = client.lock().await;

    // Get the public scratchpad data
    let result = client.get_public_scratchpad(address).await?;

    Ok(result)
}
```

## 4. Encoding

MutAnt supports different encoding methods for data stored in scratchpads.

### 4.1 Encoding Types

```rust
pub enum Encoding {
    Raw,
    Cbor,
    Json,
}
```

### 4.2 Usage

The encoding is specified when putting data:

```rust
// Store raw binary data
network.put(&pad_info, &data_chunk, Encoding::Raw, is_public).await?;

// Store structured data
network.put(&pad_info, &serialized_data, Encoding::Cbor, is_public).await?;
```

## 5. Error Handling

The Network module handles various error conditions that can occur when interacting with the Autonomi network.

### 5.1 Error Types

```rust
pub enum NetworkError {
    ClientError(String),
    ConnectionError(String),
    ScratchpadNotFound(ScratchpadAddress),
    InvalidPrivateKey(String),
    Timeout(String),
    // ... other error types
}
```

### 5.2 Error Translation

Errors from the Autonomi client are translated into MutAnt's error types:

```rust
fn translate_error(error: autonomi::Error) -> Error {
    match error {
        autonomi::Error::ScratchpadNotFound(addr) => {
            Error::NetworkError(NetworkError::ScratchpadNotFound(addr))
        },
        autonomi::Error::InvalidPrivateKey(msg) => {
            Error::NetworkError(NetworkError::InvalidPrivateKey(msg))
        },
        autonomi::Error::Timeout(msg) => {
            Error::NetworkError(NetworkError::Timeout(msg))
        },
        // ... other error translations
    }
}
```

## 6. Network Configuration

MutAnt supports different network configurations for different environments.

### 6.1 Network Choices

```rust
pub enum NetworkChoice {
    Mainnet,
    Devnet,
    Alphanet,
}
```

### 6.2 Configuration

The network choice is specified during initialization:

```rust
// Initialize with mainnet
let mutant = MutAnt::init(private_key_hex).await?;

// Initialize with devnet
let mutant = MutAnt::init_local().await?;

// Initialize with alphanet
let mutant = MutAnt::init_alphanet(private_key_hex).await?;
```

## 7. Master Index Handling

The Network module is responsible for loading and saving the Master Index.

### 7.1 Loading the Master Index

```rust
pub async fn load_master_index(
    &self,
    address: &ScratchpadAddress,
    private_key: &[u8],
) -> Result<MasterIndex, Error> {
    let client = self.get_client().await?;
    let client = client.lock().await;

    // Get the master index scratchpad
    let result = client.get_scratchpad(address, private_key).await?;

    // Deserialize the master index
    let master_index: MasterIndex = serde_cbor::from_slice(&result.data)?;

    Ok(master_index)
}
```

### 7.2 Saving the Master Index

```rust
pub async fn save_master_index(
    &self,
    address: &ScratchpadAddress,
    private_key: &[u8],
    master_index: &MasterIndex,
) -> Result<PutResult, Error> {
    let client = self.get_client().await?;
    let client = client.lock().await;

    // Serialize the master index
    let data = serde_cbor::to_vec(master_index)?;

    // Update the master index scratchpad
    let result = client.update_scratchpad(
        address,
        private_key,
        &data,
        Encoding::Cbor,
        false,
    ).await?;

    Ok(result)
}
```