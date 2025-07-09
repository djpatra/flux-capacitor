use crate::MessageEnum;
use crossbeam::channel::{Receiver, Sender};

pub struct Processing {
    rx: Receiver<MessageEnum>,
    tx: Sender<MessageEnum>,
}

impl Processing {
    pub fn new(rx: Receiver<MessageEnum>, tx: Sender<MessageEnum>) -> Self {
        Self { rx, tx }
    }

    pub async fn start(&mut self) {
        loop {
            while let Ok(message) = self.rx.try_recv() {
                // TODO:

                if let Err(e) = self.tx.send(message) {
                    println!("Transmitter channel closed: {}", e);
                }
            }

            tokio::task::yield_now().await;
        }
    }
}
