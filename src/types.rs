use crate::{MessageTrait, MessageType, MessageWrapperTrait};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageData {
    pub parent_signature: Option<Vec<u8>>,
    pub message_type: u8,
    pub points_value: u16,
    pub ts: u64,
}

impl MessageData {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        bincode::deserialize(bytes).unwrap()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Message {
    pub data: MessageData,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MessageEnum {
    MyMessageType(Message),
}

impl MessageTrait for MessageEnum {
    fn from_bytes(bytes: &[u8]) -> Self {
        bincode::deserialize(bytes).unwrap()
    }

    fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }

    fn get_type(&self) -> MessageType {
        match self {
            MessageEnum::MyMessageType(message) => message.data.message_type,
        }
    }

    fn get_signature(&self) -> &Vec<u8> {
        match self {
            MessageEnum::MyMessageType(message) => &message.signature,
        }
    }

    fn get_parent_signature(&self) -> Option<&Vec<u8>> {
        match self {
            MessageEnum::MyMessageType(message) => message.data.parent_signature.as_ref(),
        }
    }

    fn get_points_value(&self) -> u16 {
        match self {
            MessageEnum::MyMessageType(message) => message.data.points_value,
        }
    }

    fn get_ts(&self) -> u64 {
        match self {
            MessageEnum::MyMessageType(message) => message.data.ts,
        }
    }

    fn from_values(message_data: MessageData, signature: Vec<u8>) -> Self {
        MessageEnum::MyMessageType(Message {
            data: message_data,
            signature,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MessageWrapperEnum {
    MyMessageWrapperType([MessageEnum; 1]),
}

impl MessageWrapperTrait for MessageWrapperEnum {
    fn from_bytes(bytes: &[u8]) -> Vec<MessageEnum> {
        let message = MessageEnum::from_bytes(bytes);
        vec![message]
    }

    fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }
}
