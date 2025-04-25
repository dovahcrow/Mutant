#[derive(Debug, Clone)]
pub enum MessageType {
    Ping,
    Pong,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub message_type: MessageType,
}

impl Message {
    pub fn new(message_type: MessageType) -> Self {
        Self { message_type }
    }
}
