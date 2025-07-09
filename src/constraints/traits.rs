use crate::{MessageData, MessageEnum};

pub type MessageType = u8;

pub trait MessageTrait: Sized {
    fn from_bytes(bytes: &[u8]) -> Self;
    fn to_bytes(&self) -> Vec<u8>;
    fn get_type(&self) -> MessageType;
    fn get_signature(&self) -> &Vec<u8>;
    fn get_parent_signature(&self) -> Option<&Vec<u8>>;
    fn get_points_value(&self) -> u16;
    fn from_values(message_data: MessageData, signature: Vec<u8>) -> Self;
    fn get_ts(&self) -> u64;
}

pub trait MessageWrapperTrait: Sized {
    fn from_bytes(bytes: &[u8]) -> Vec<MessageEnum>;
    fn to_bytes(&self) -> Vec<u8>;
}
