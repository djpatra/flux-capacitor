use crate::{MessageEnum, MessageTrait, MessageType};
use ringbuffer::{ConstGenericRingBuffer, RingBuffer};

pub struct PointsManager {
    recent_signature_history: ConstGenericRingBuffer<Vec<u8>, 100>,
    recent_type_history: ConstGenericRingBuffer<MessageType, 100>,
    dependency_length: u64,
}

impl PointsManager {
    pub fn new() -> Self {
        Self {
            recent_signature_history: ConstGenericRingBuffer::new(),
            recent_type_history: ConstGenericRingBuffer::new(),
            dependency_length: 0,
        }
    }

    pub fn get_message_points(&mut self, message: MessageEnum) -> u64 {
        // Check if the message has a parent and if it is in the history
        if let Some(parent_signature) = message.get_parent_signature() {
            if !self.recent_signature_history.contains(parent_signature) {
                self.dependency_length = 0;
                return 0;
            } else {
                self.dependency_length += 1;
            }
        } else {
            self.dependency_length = 0;
        }

        // Check if the message is in the history
        if self
            .recent_signature_history
            .iter()
            .filter(|id| *id == message.get_signature())
            .count()
            > 1
        {
            return 0;
        }

        // Message is not in recent history and meets potential parent criteria, add it to the history
        self.recent_signature_history
            .push(message.get_signature().clone());
        self.recent_type_history.push(message.get_type());

        let mut default_bonus_multiplier = 4;

        // Check if we have 3 in a row of the same type
        let last_two: Vec<_> = self.recent_type_history.iter().rev().take(3).collect();
        if last_two.len() >= 3 && last_two.into_iter().all(|t| *t == message.get_type()) {
            default_bonus_multiplier /= 2;
        }

        // Check if we have 3 in a row of different types
        let last_three: Vec<_> = self.recent_type_history.iter().rev().take(3).collect();
        let unique_types: std::collections::HashSet<MessageType> =
            last_three.into_iter().cloned().collect();
        if unique_types.len() >= 3 {
            default_bonus_multiplier *= 2;
        }

        // Points = message value * type bonus multiplier + dependency length
        let total_points =
            message.get_points_value() as u64 * default_bonus_multiplier + self.dependency_length;

        return total_points;
    }
}
