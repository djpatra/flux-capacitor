use std::collections::HashSet;

use crate::{
    MessageData, MessageEnum, MessageTrait, MessageWrapperEnum, MessageWrapperTrait, PointsManager,
};
use crossbeam::channel::Receiver;
use sha2::{Digest, Sha256, Sha384, Sha512};
use tokio::time::{Duration, interval};

#[derive(Debug, Clone, Copy, Default)]
struct Metrics {
    received: u64,
    valid: u64,
    invalid: u64,
    duplicates: u64,
    points: u64,
    points_per_second: u64,
}

pub struct Sink {
    rx: Receiver<Vec<u8>>,
    points_manager: PointsManager,
    metrics: Metrics,
    metrics_tick_index: u64,
    signatures: HashSet<Vec<u8>>,
}

impl Sink {
    const DURATION_TICKS: u64 = 10;

    pub fn new(rx: Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            points_manager: PointsManager::new(),
            metrics: Metrics::default(),
            metrics_tick_index: 0,
            signatures: HashSet::new(),
        }
    }

    fn validate_message_signature(&self, message: &MessageEnum) -> bool {
        // Get the hash algorithm based on message type
        let hasher: fn(&[u8]) -> Vec<u8> = match message.get_type() {
            0 => |data| Sha256::digest(data).to_vec(), // Red - SHA256
            1 => |data| Sha384::digest(data).to_vec(), // Yellow - SHA384
            2 => |data| Sha512::digest(data).to_vec(), // Blue - SHA512
            _ => return false,                         // Unknown message type
        };

        // Reconstruct the message data
        let message_data = MessageData {
            parent_signature: message.get_parent_signature().cloned(),
            message_type: message.get_type(),
            points_value: message.get_points_value(),
            ts: message.get_ts(),
        };

        // Calculate the expected signature
        let data_bytes = message_data.to_bytes();
        let expected_signature = hasher(&data_bytes);

        // Compare with the actual signature
        expected_signature == *message.get_signature()
    }

    pub async fn start(&mut self) {
        let mut metrics_ticker = interval(Duration::from_secs(1));

        loop {
            tokio::select! {
                _ = metrics_ticker.tick() => {
                    if self.metrics_tick_index != 0 {
                        println!("Metrics: received={}, valid={}, invalid={}, duplicates={}, total_points={}, points_per_second={}", self.metrics.received, self.metrics.valid, self.metrics.invalid, self.metrics.duplicates, self.metrics.points, self.metrics.points_per_second);
                    }
                    self.metrics_tick_index += 1;
                    self.metrics.points_per_second = 0;
                    if self.metrics_tick_index >= Self::DURATION_TICKS {
                        println!("Total points: {}", self.metrics.points);
                        break;
                    }
                }
                _ = tokio::task::yield_now() => {
                    if let Ok(data) = self.rx.try_recv() {
                        let messages = MessageWrapperEnum::from_bytes(&data);
                        for message in messages {
                            self.metrics.received += 1;
                            if self.validate_message_signature(&message) {
                                if !self.signatures.contains(message.get_signature()) {
                                    self.metrics.valid += 1;
                                    self.signatures.insert(message.get_signature().clone());
                                    let points = self.points_manager.get_message_points(message);
                                    self.metrics.points += points;
                                    self.metrics.points_per_second += points;
                                } else {
                                    self.metrics.duplicates += 1;
                                }
                            } else {
                                self.metrics.invalid += 1;
                            }
                        }
                    }
                }
            }
        }
    }
}
