use crate::{MessageData, MessageEnum, MessageTrait};
use crossbeam::channel::Sender;
use rand::Rng;
use ringbuffer::{AllocRingBuffer, RingBuffer};
use tokio::time::{Duration, interval};

use sha2::{Digest, Sha256, Sha384, Sha512};

const HISTORY_SIZE: usize = 100_000;

struct TypeParams {
    message_type: u8,
    delay_micros: u64,
    points_value_min: u16,
    points_value_max: u16,
    percent_chance_parent_dependency: u8,
    hasher: fn(&[u8]) -> Vec<u8>,
    signatures: AllocRingBuffer<Vec<u8>>,
}

impl TypeParams {
    pub fn generate_message(&mut self) -> MessageEnum {
        let has_parent =
            rand::rng().random_bool(self.percent_chance_parent_dependency as f64 / 100.0);
        let points_value = rand::rng().random_range(self.points_value_min..=self.points_value_max);
        let parent_signature = if has_parent && self.signatures.len() > 0 {
            let random_index = rand::rng().random_range(0..self.signatures.len());
            Some(self.signatures.iter().nth(random_index).unwrap().clone())
        } else {
            None
        };

        let data = MessageData {
            parent_signature,
            message_type: self.message_type,
            points_value,
            ts: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
        };

        let data_bytes = data.to_bytes();
        let signature = (self.hasher)(&data_bytes);

        self.signatures.push(signature.clone());

        MessageEnum::from_values(data, signature)
    }
}

pub struct Source {
    tx: Sender<MessageEnum>,
}

impl Source {
    pub fn new(tx: Sender<MessageEnum>) -> Self {
        Self { tx }
    }

    pub async fn start(&mut self) {
        let mut red_params = TypeParams {
            message_type: 0,
            delay_micros: 50,
            points_value_min: 5,
            points_value_max: 10,
            percent_chance_parent_dependency: 90,
            hasher: |data| Sha256::digest(data).to_vec(), // Generates 32 byte length hash
            signatures: AllocRingBuffer::new(HISTORY_SIZE),
        };

        let mut yellow_params = TypeParams {
            message_type: 1,
            delay_micros: 100,
            points_value_min: 10,
            points_value_max: 20,
            percent_chance_parent_dependency: 95,
            hasher: |data| Sha384::digest(data).to_vec(), // Generates 48 byte length hash
            signatures: AllocRingBuffer::new(HISTORY_SIZE),
        };

        let mut blue_params = TypeParams {
            message_type: 2,
            delay_micros: 200,
            points_value_min: 20,
            points_value_max: 50,
            percent_chance_parent_dependency: 99,
            hasher: |data| Sha512::digest(data).to_vec(), // Generates 64 byte length hash
            signatures: AllocRingBuffer::new(HISTORY_SIZE),
        };

        let mut red_ticker = interval(Duration::from_micros(red_params.delay_micros));
        let mut yellow_ticker = interval(Duration::from_micros(yellow_params.delay_micros));
        let mut blue_ticker = interval(Duration::from_micros(blue_params.delay_micros));

        loop {
            tokio::select! {
                _ = red_ticker.tick() => {
                    let message = red_params.generate_message();
                    let _ = self.tx.send(message);
                }
                _ = yellow_ticker.tick() => {
                    let message = yellow_params.generate_message();
                    let _ = self.tx.send(message);
                }
                _ = blue_ticker.tick() => {
                    let message = blue_params.generate_message();
                    let _ = self.tx.send(message);
                }
            }

            tokio::task::yield_now().await;
        }
    }
}
