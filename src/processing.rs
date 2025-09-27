use crate::{MessageEnum, MessageTrait};
use crossbeam::channel::{Receiver, Sender};
use std::cmp::Reverse;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
struct MessageInfo {
    message: MessageEnum,
    size_bytes: usize,
    point_estimate: u64,
}

#[derive(Debug, Clone)]
struct ProcessingState {
    // Transmitted messages; enables to predict the sink state
    // Last 100 sent (matches PointsManager history)
    sent_signatures: VecDeque<Vec<u8>>,
    // Last few sent types
    sent_types: VecDeque<u8>,

    // Signatures of the ingested messages which are yet to be sent
    accumulated_signatures: HashSet<Vec<u8>>,

    // Current batch to be transmitted
    current_batch: Vec<MessageEnum>,
    current_batch_size: usize,

    // Message queues
    // Type 0 - 32 bytes
    red_queue: Vec<MessageInfo>,
    // Type 1 - 48 bytes
    yellow_queue: Vec<MessageInfo>,
    // Type 2 - 64 bytes
    blue_queue: Vec<MessageInfo>,

    // Deduplication
    seen_signatures: HashSet<Vec<u8>>,

    // Performance metrics
    total_processed: u64,
    total_sent: u64,
    total_rejected: u64,
    optimal_triplets_sent: u64,
}

pub struct Processing {
    rx: Receiver<MessageEnum>,
    tx: Sender<MessageEnum>,
    state: ProcessingState,
    last_metrics_time: std::time::Instant,
}

impl Processing {
    const MAX_BATCH_SIZE: usize = 4096;
    const SINK_HISTORY_SIZE: usize = 100;
    // To pattern match the types of consecutive messages sent
    // Keeping size 8 to make it cache friendly
    const TYPE_PATTERN_WINDOW: usize = 8;
    // 4096 / 32 = 128 is the maximum number of messages we can fill in a batch.
    // We will keep the number of messages that need to be ingested in a short
    // ingest burst 10X of this value, so that, we ensure that we have seen enough
    // messages of each type
    const MAX_INGEST_PER_CYCLE: usize = 10 * 128;

    pub fn new(rx: Receiver<MessageEnum>, tx: Sender<MessageEnum>) -> Self {
        Self {
            rx,
            tx,
            state: ProcessingState {
                sent_signatures: VecDeque::with_capacity(Self::SINK_HISTORY_SIZE),
                sent_types: VecDeque::with_capacity(Self::TYPE_PATTERN_WINDOW),
                accumulated_signatures: HashSet::new(),
                current_batch: Vec::new(),
                current_batch_size: 0,
                red_queue: Vec::new(),
                yellow_queue: Vec::new(),
                blue_queue: Vec::new(),
                seen_signatures: HashSet::new(),
                total_processed: 0,
                total_sent: 0,
                total_rejected: 0,
                optimal_triplets_sent: 0,
            },
            last_metrics_time: std::time::Instant::now(),
        }
    }

    pub async fn start(&mut self) {
        let mut batch_ticker = tokio::time::interval(tokio::time::Duration::from_millis(10));

        let should_optimize = Arc::new(AtomicBool::new(false));
        let should_optimize_clone = should_optimize.clone();

        loop {
            tokio::select! {
                    _ = async {
                    loop {
                    // Does timer wants ingestion to stop and optimize
                    if should_optimize_clone.load(Ordering::Relaxed) {
                        break;
                    }

                    let ingested = self.ingest_messages_batch();
                    self.state.total_processed += ingested;

                    // Yield to allow timer to signal
                    tokio::task::yield_now().await;
                }
            } => {
                // Timer has signalled; Optimize the batch now
                self.handle_timer_signal();
                should_optimize.store(false, Ordering::Relaxed);
            }

            _ = batch_ticker.tick() => {
                // Signal ingestion to stop and start optimization
                should_optimize.store(true, Ordering::Relaxed);

                // FixMe: Add a stop criteria. Currently, processing stops when the main exits.
            }
            }
        }
    }

    // Handle the timer signal - optimize and flush batch
    fn handle_timer_signal(&mut self) {
        // Optimize the betch
        if !self.all_queues_empty() {
            self.optimize_current_batch();
        }

        // Send the batch if we have messages
        if !self.state.current_batch.is_empty() {
            self.flush_batch();
        }
    }

    /// Ingest messages in a batch (limited by MAX_INGEST_PER_CYCLE). This ensures that we have
    /// accumulated enough messages that we can optimize the ordering for maximum points and create
    /// a batch for transmission
    fn ingest_messages_batch(&mut self) -> u64 {
        let mut count = 0;

        while count < Self::MAX_INGEST_PER_CYCLE {
            match self.rx.try_recv() {
                Ok(message) => {
                    // Skip duplicates immediately
                    if self
                        .state
                        .seen_signatures
                        .insert(message.get_signature().clone())
                    {
                        count += 1;

                        let size_bytes = message.to_bytes().len();
                        let parent_signature = message.get_parent_signature().cloned();
                        let point_estimate = self.estimate_message_points(&message);
                        let msg_info = MessageInfo {
                            message,
                            size_bytes,
                            point_estimate,
                        };

                        // Doesthis message has a parent dependency
                        if let Some(ref parent_sig) = parent_signature {
                            if self.state.sent_signatures.contains(parent_sig) {
                                // Parent is in our recent sent history - message is viable
                                self.state
                                    .accumulated_signatures
                                    .insert(msg_info.message.get_signature().clone());

                                self.store_message_by_type(msg_info);
                            } else {
                                // Parent not in recent history - child will score 0 points
                                // No point storing it since parent was evicted from sink history
                                // FixiMe: the behaviour in PointsManager to retain messages with dangling parents
                                self.state.total_rejected += 1;
                                continue;
                            }
                        } else {
                            // No parent dependency - immediately viable
                            self.state
                                .accumulated_signatures
                                .insert(msg_info.message.get_signature().clone());

                            self.store_message_by_type(msg_info);
                        }
                    }
                }
                Err(_) => break, // No more messages available right now
            }
        }

        count as u64
    }

    #[inline]
    fn store_message_by_type(&mut self, msg_info: MessageInfo) {
        match msg_info.message.get_type() {
            0 => self.state.red_queue.push(msg_info),
            1 => self.state.yellow_queue.push(msg_info),
            2 => self.state.blue_queue.push(msg_info),
            _ => {} // Invalid type
        }
    }

    /// Estimate points this message would get based on current predicted sink state
    #[inline]
    fn estimate_message_points(&self, message: &MessageEnum) -> u64 {
        // Base points from message
        let base_points = message.get_points_value() as u64;

        // Predict multiplier based on type sequence if we send this message next
        // Small data structure. So, no harm in cloning
        let mut future_types = self.state.sent_types.clone();
        future_types.push_back(message.get_type());

        let multiplier = self.calculate_multiplier_for_sequence(&future_types);

        // Add dependency length bonus
        // FixMe: The dependency calculation in PointsManager is cumulative across all messages.
        // It should be calculated for a single message hierarchy (parent-child relationship)
        // We are keeping this calculation simple now. This needs to change when the dependency
        // length calculation in PointsManager change
        let dependency_bonus = if message.get_parent_signature().is_some() {
            self.estimate_dependency_length()
        } else {
            0
        };

        base_points * multiplier + dependency_bonus
    }

    /// Calculates the multiplier for points based on the implementation in PointsManager
    #[inline]
    fn calculate_multiplier_for_sequence(&self, type_sequence: &VecDeque<u8>) -> u64 {
        let mut multiplier = 4u64; // Default

        if type_sequence.len() >= 3 {
            // Look at last 3 types
            let last_three: Vec<u8> = type_sequence.iter().rev().take(3).copied().collect();
            let unique_types: HashSet<u8> = last_three.iter().copied().collect();

            if unique_types.len() >= 3 {
                multiplier *= 2; // All different -> 8x total
            } else if unique_types.len() == 1 {
                multiplier /= 2; // All same -> 2x total
            }
            // unique_types.len() == 2 keeps default 4x
        }

        multiplier
    }

    // Simplified estimation of dependency length
    #[inline]
    fn estimate_dependency_length(&self) -> u64 {
        1
    }

    #[inline]
    fn can_form_triplet(&self) -> bool {
        !self.state.red_queue.is_empty()
            && !self.state.yellow_queue.is_empty()
            && !self.state.blue_queue.is_empty()
    }

    /// Optimization engine that implements the point-maximization strategy for the time based scoring window.
    /// This function transforms the queued messages into an ordered batch that maximizes points at the sink
    /// by creating triplets with 2x multipliers, prioritizing high-value messages, and respecting parent-child
    /// relationship.
    /// It works in 3 steps:
    /// 1. Perfect triplet formation [Exactly one message of each type (Red, Yellow, Blue)]
    /// 2. Remaining space optimization
    /// 3. Recalculation of points for the remaining messages
    fn optimize_current_batch(&mut self) {
        // Sort each queue by estimated points
        self.state
            .red_queue
            .sort_by_key(|m| Reverse(m.point_estimate));
        self.state
            .yellow_queue
            .sort_by_key(|m| Reverse(m.point_estimate));
        self.state
            .blue_queue
            .sort_by_key(|m| Reverse(m.point_estimate));

        // Step 1: Perfect triplets first (maximum 2x multiplier)
        while self.can_form_triplet() {
            if let Some(triplet) = self.form_perfect_triplet() {
                let triplet_size: usize = triplet.iter().map(|m| m.to_bytes().len()).sum();

                if self.state.current_batch_size + triplet_size <= Self::MAX_BATCH_SIZE {
                    for message in triplet {
                        self.add_to_batch(message);
                    }
                    self.state.optimal_triplets_sent += 1;
                } else {
                    // Space over or triplet wont fit
                    break;
                }
            } else {
                break;
            }
        }

        // Step 2: If the batch is not filled with triplets, then try high-value singles
        if self.state.current_batch.is_empty() || self.has_batch_space() {
            self.pack_remaining_space_optimally();
        }

        // Step 3: Re calculate points for remaining messages
        self.update_remaining_message_estimates();
    }

    /// Find the globally optimal triplet in 3 steps
    /// 1. All 3 different types (for 2x multiplier)
    /// 2. Highest combined estimated value
    /// 3. Valid parent-child relationship
    fn form_perfect_triplet(&mut self) -> Option<Vec<MessageEnum>> {
        let mut best_triplet = None;
        let mut best_score = 0u64;
        let mut best_removal_indices = None;

        // Try all the 6 possible orderings
        let orderings: [[u8; 3]; 6] = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];

        for &ordering in &orderings {
            if let Some((triplet, removal_indices, chain_score)) =
                self.try_form_triplet_with_order(ordering)
            {
                if chain_score > best_score {
                    best_score = chain_score;
                    best_triplet = Some(triplet);
                    best_removal_indices = Some(removal_indices);
                }
            }
        }

        if let (Some(triplet), Some(removal_indices)) = (best_triplet, best_removal_indices) {
            // Remove these messages from queues by index (in reverse order to avoid shifting)
            let mut sorted_indices = removal_indices;
            sorted_indices.sort_by_key(|(_, idx)| std::cmp::Reverse(*idx)); // Remove highest indices first

            for (queue_type, idx) in sorted_indices {
                match queue_type {
                    0 => {
                        self.state.red_queue.remove(idx);
                    }
                    1 => {
                        self.state.yellow_queue.remove(idx);
                    }
                    2 => {
                        self.state.blue_queue.remove(idx);
                    }
                    _ => {}
                }
            }

            Some(triplet)
        } else {
            None
        }
    }

    /// Try to form a triplet of messages (Red, Yellow,Blue) in the order provided to maximize
    /// the 2x multiplier bonus from the PointManager scoring system and also prioritising
    /// dependency bonus
    fn try_form_triplet_with_order(
        &self,
        order: [u8; 3],
    ) -> Option<(Vec<MessageEnum>, Vec<(u8, usize)>, u64)> {
        let mut triplet = Vec::new();
        let mut removal_indices = Vec::new();
        let mut total_score = 0u64;
        let mut batch_signatures = HashSet::new(); // Track signatures in this potential triplet

        for &msg_type in &order {
            let queue = match msg_type {
                0 => &self.state.red_queue,
                1 => &self.state.yellow_queue,
                2 => &self.state.blue_queue,
                _ => continue,
            };

            // Find best message considering internal dependencies
            let (selected_message, queue_index, dependency_bonus) =
                self.select_best_message_for_chain(queue, &batch_signatures);

            batch_signatures.insert(selected_message.get_signature().clone());
            triplet.push(selected_message.clone());
            removal_indices.push((msg_type, queue_index));

            let base_points = selected_message.get_points_value() as u64;
            let message_score = base_points + dependency_bonus;
            total_score += message_score;
        }

        if triplet.len() == 3 {
            // Order the triplet to respect parent-child relationships
            let ordered_triplet = self.order_by_dependencies(triplet);
            Some((ordered_triplet, removal_indices, total_score))
        } else {
            None
        }
    }

    /// Order messages to ensure parents come before children
    fn order_by_dependencies(&self, mut messages: Vec<MessageEnum>) -> Vec<MessageEnum> {
        let mut ordered = Vec::new();
        let mut remaining: Vec<_> = messages.drain(..).collect();
        let mut signatures_added = HashSet::new();

        // Add messages with dependencies already satisfied first
        while !remaining.is_empty() {
            let mut progress = false;

            for i in (0..remaining.len()).rev() {
                let message = &remaining[i];
                let can_add = if let Some(parent_sig) = message.get_parent_signature() {
                    // Check if parent is already in ordered list or sent history
                    signatures_added.contains(parent_sig)
                        || self.state.sent_signatures.contains(parent_sig)
                } else {
                    // No parent dependency
                    true
                };

                if can_add {
                    let msg = remaining.remove(i);
                    signatures_added.insert(msg.get_signature().clone());
                    ordered.push(msg);
                    progress = true;
                }
            }

            // If no progress, add remaining messages anyway
            if !progress {
                ordered.extend(remaining.drain(..));
                break;
            }
        }

        ordered
    }

    /// Select the best message from a queue considering dependency bonus
    fn select_best_message_for_chain(
        &self,
        queue: &[MessageInfo],
        batch_signatures: &HashSet<Vec<u8>>,
    ) -> (MessageEnum, usize, u64) {
        // First, look for messages whose parent is in the current batch
        for (idx, msg_info) in queue.iter().enumerate() {
            if let Some(parent_sig) = msg_info.message.get_parent_signature() {
                if batch_signatures.contains(parent_sig) {
                    // Parent is in current batch - high dependency bonus
                    return (msg_info.message.clone(), idx, 10);
                }
            }
        }

        // Second, look for messages whose parent is in sent history
        for (idx, msg_info) in queue.iter().enumerate() {
            if self.message_extends_chain(&msg_info.message) {
                let dependency_bonus = self.calculate_dependency_bonus(&msg_info.message);
                return (msg_info.message.clone(), idx, dependency_bonus);
            }
        }

        // No dependencies found, take highest-value message
        let best_msg = &queue[0];
        (best_msg.message.clone(), 0, 0)
    }

    /// Check if a message extends an existing dependency chain
    #[inline]
    fn message_extends_chain(&self, message: &MessageEnum) -> bool {
        if let Some(parent_sig) = message.get_parent_signature() {
            self.state.sent_signatures.contains(parent_sig)
        } else {
            false
        }
    }

    /// Calculate dependency bonus for a message based on its chain position
    #[inline]
    fn calculate_dependency_bonus(&self, message: &MessageEnum) -> u64 {
        if let Some(parent_sig) = message.get_parent_signature()
            && self.state.sent_signatures.contains(parent_sig)
        {
            // Find parent's position in sent history (more recent = longer chain)
            for (depth, sent_sig) in self.state.sent_signatures.iter().rev().enumerate() {
                if sent_sig == parent_sig {
                    return (depth + 1) as u64;
                }
            }
            1
        } else {
            0
        }
    }

    /// Removes message from the respective queue
    /// FixMe: This can be a potential problem as the function is doing linear search
    /// With the given rate of sourcing and transmission of messages, there will
    /// be about 10K messages in each queue in the steady state. Linear search over
    /// 10K messages (of size ~130 bytes) should not be an issue. But this is one area
    /// of improvement
    fn remove_message_from_queue(&mut self, message: &MessageEnum) {
        match message.get_type() {
            0 => self
                .state
                .red_queue
                .retain(|m| m.message.get_signature() != message.get_signature()),
            1 => self
                .state
                .yellow_queue
                .retain(|m| m.message.get_signature() != message.get_signature()),
            2 => self
                .state
                .blue_queue
                .retain(|m| m.message.get_signature() != message.get_signature()),
            _ => {}
        }
    }

    /// Collect all valid remaining messages
    fn pack_remaining_space_optimally(&mut self) {
        let mut all_valid = Vec::new();

        for msg_info in &self.state.red_queue {
            all_valid.push(msg_info.clone());
        }
        for msg_info in &self.state.yellow_queue {
            all_valid.push(msg_info.clone());
        }
        for msg_info in &self.state.blue_queue {
            all_valid.push(msg_info.clone());
        }

        // Sort by points-per-byte ratio for efficient packing
        all_valid.sort_by(|a, b| {
            let ratio_a = a.point_estimate as f64 / a.size_bytes as f64;
            let ratio_b = b.point_estimate as f64 / b.size_bytes as f64;
            ratio_b
                .partial_cmp(&ratio_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Pack greedily
        for msg_info in all_valid {
            if self.state.current_batch_size + msg_info.size_bytes <= Self::MAX_BATCH_SIZE {
                let message = msg_info.message.clone();
                self.add_to_batch(message);
                self.remove_message_from_queue(&msg_info.message);
            }
        }
    }

    // Calculate estimated points for all queued messages since the sent history changed
    // after the current batch was finalized. We need to collect updates first to avoid borrow checker issues
    // FixMe: This iis also a potential area for improvement
    fn update_remaining_message_estimates(&mut self) {
        let mut red_updates = Vec::new();
        let mut yellow_updates = Vec::new();
        let mut blue_updates = Vec::new();

        // Collect all messages that need point recalculation
        for (i, msg_info) in self.state.red_queue.iter().enumerate() {
            let new_points = self.estimate_message_points(&msg_info.message);
            red_updates.push((i, new_points));
        }

        for (i, msg_info) in self.state.yellow_queue.iter().enumerate() {
            let new_points = self.estimate_message_points(&msg_info.message);
            yellow_updates.push((i, new_points));
        }

        for (i, msg_info) in self.state.blue_queue.iter().enumerate() {
            let new_points = self.estimate_message_points(&msg_info.message);
            blue_updates.push((i, new_points));
        }

        // Apply the updates
        for (i, new_points) in red_updates {
            self.state.red_queue[i].point_estimate = new_points;
        }

        for (i, new_points) in yellow_updates {
            self.state.yellow_queue[i].point_estimate = new_points;
        }

        for (i, new_points) in blue_updates {
            self.state.blue_queue[i].point_estimate = new_points;
        }
    }

    /// Add a message to the current batch
    fn add_to_batch(&mut self, message: MessageEnum) {
        let size_bytes = message.to_bytes().len();
        self.state.current_batch_size += size_bytes;

        let signature = message.get_signature();
        self.state.accumulated_signatures.remove(signature);

        self.state.sent_signatures.push_back(signature.clone());

        // Evict oldest message
        if self.state.sent_signatures.len() > Self::SINK_HISTORY_SIZE {
            self.state.sent_signatures.pop_front();
        }

        self.state.sent_types.push_back(message.get_type());

        // Evict type of the old message
        if self.state.sent_types.len() > Self::TYPE_PATTERN_WINDOW {
            self.state.sent_types.pop_front();
        }

        self.state.current_batch.push(message);
    }

    #[inline]
    fn all_queues_empty(&self) -> bool {
        self.state.red_queue.is_empty()
            && self.state.yellow_queue.is_empty()
            && self.state.blue_queue.is_empty()
    }

    #[inline]
    fn has_batch_space(&self) -> bool {
        self.state.current_batch_size < Self::MAX_BATCH_SIZE
    }

    /// Transmit the current batch and print perf metrics
    fn flush_batch(&mut self) {
        for message in self.state.current_batch.drain(..) {
            self.state.total_sent += 1;

            if let Err(e) = self.tx.send(message) {
                eprintln!("Transmitter channel closed: {}", e);
                return;
            }
        }

        self.state.current_batch_size = 0;
        let elapsed = self.last_metrics_time.elapsed();

        // Performance metrics
        if elapsed >= std::time::Duration::from_secs(2) {
            println!(
                "Processing: {} processed  |  {} sent  | {} rejected  | {} triplets",
                self.state.total_processed,
                self.state.total_sent,
                self.state.total_rejected,
                self.state.optimal_triplets_sent,
            );

            self.last_metrics_time = std::time::Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MessageData, MessageEnum, MessageTrait};

    fn create_test_message(
        msg_type: u8,
        points_value: u16,
        parent_sig: Option<Vec<u8>>,
    ) -> MessageEnum {
        let data = MessageData {
            parent_signature: parent_sig,
            message_type: msg_type,
            points_value,
            ts: 12345,
        };

        let signature = format!("xxxx_{}_{}", msg_type, points_value).into_bytes();

        MessageEnum::from_values(data, signature)
    }

    fn create_test_processing() -> Processing {
        let (tx, _rx) = crossbeam::channel::unbounded();
        let (_source_tx, source_rx) = crossbeam::channel::unbounded();
        Processing::new(source_rx, tx)
    }

    #[test]
    fn test_optimize_perfect_triplet_formation() {
        let mut processing = create_test_processing();

        // Add messages of all three types
        let red_high = create_test_message(0, 10, None); // High value, no parent
        let red_med = create_test_message(0, 8, None); // Medium value, no parent
        let red_low = create_test_message(0, 5, Some(b"00000000".to_vec())); // Low value, invalid parent

        let yellow_high = create_test_message(1, 20, None);
        let yellow_low = create_test_message(1, 12, None);

        let blue_high = create_test_message(2, 50, None);
        let blue_med = create_test_message(2, 35, None);
        let blue_low = create_test_message(2, 30, None);

        processing.store_message_by_type(MessageInfo {
            message: red_high.clone(),
            size_bytes: 120,
            point_estimate: 40, // 10 * 4
        });
        processing.store_message_by_type(MessageInfo {
            message: red_med.clone(),
            size_bytes: 120,
            point_estimate: 32, // 8 * 4
        });
        processing.store_message_by_type(MessageInfo {
            message: red_low.clone(),
            size_bytes: 120,
            point_estimate: 0, // 0
        });
        processing.store_message_by_type(MessageInfo {
            message: yellow_high.clone(),
            size_bytes: 136,
            point_estimate: 80, // 20 * 4
        });
        processing.store_message_by_type(MessageInfo {
            message: yellow_low.clone(),
            size_bytes: 136,
            point_estimate: 48, // 12 * 4
        });
        processing.store_message_by_type(MessageInfo {
            message: blue_high.clone(),
            size_bytes: 152,
            point_estimate: 200, // 50 * 4
        });
        processing.store_message_by_type(MessageInfo {
            message: blue_med.clone(),
            size_bytes: 152,
            point_estimate: 140, // 35 * 4
        });
        processing.store_message_by_type(MessageInfo {
            message: blue_low.clone(),
            size_bytes: 4000,    // Large, This should not be put into the batch
            point_estimate: 140, // 35 * 4
        });
        processing.optimize_current_batch();

        let batch_messages = &processing.state.current_batch;

        // Should have selected red_high
        assert_eq!(
            batch_messages[0].get_signature(),
            red_high.get_signature(),
            "First message should be red_high"
        );

        // Should have selected yellow_high
        assert_eq!(
            batch_messages[1].get_signature(),
            yellow_high.get_signature(),
            "Second message should be yellow_high"
        );

        // Should have selected blue_high
        assert_eq!(
            batch_messages[2].get_signature(),
            blue_high.get_signature(),
            "Third message should be blue_high"
        );

        // Selected messages were removed from queues
        assert_eq!(
            processing.state.red_queue.len(),
            0,
            "Red queue should be empty"
        );
        assert_eq!(
            processing.state.yellow_queue.len(),
            0,
            "Yellow queue should be empty"
        );
        assert_eq!(
            processing.state.blue_queue.len(),
            1,
            "Blue message should be removed"
        );

        // Large blue_low is remaining
        assert!(
            processing
                .state
                .blue_queue
                .iter()
                .any(|m| m.message.get_signature() == blue_low.get_signature())
        );
    }

    #[test]
    fn test_optimize_with_parent_child_reln_and_batch_limits() {
        let mut processing = create_test_processing();

        let parent_sig = b"11111".to_vec();
        processing
            .state
            .sent_signatures
            .push_back(parent_sig.clone());

        let red_parent = create_test_message(0, 8, None);
        let red_child = create_test_message(0, 9, Some(parent_sig.clone()));

        let yellow_valid = create_test_message(1, 15, None);
        let yellow_child = create_test_message(1, 18, Some(parent_sig.clone()));

        let blue_valid = create_test_message(2, 40, None);
        let blue_large = create_test_message(2, 45, None);

        // Add messages to queues
        processing.store_message_by_type(MessageInfo {
            message: red_parent.clone(),
            size_bytes: 120,
            point_estimate: 32, // 8 * 4
        });
        processing.store_message_by_type(MessageInfo {
            message: red_child.clone(),
            size_bytes: 120,
            point_estimate: 37, // 9 * 4 + 1
        });
        processing.store_message_by_type(MessageInfo {
            message: yellow_valid.clone(),
            size_bytes: 136,
            point_estimate: 60, // 15 * 4
        });
        processing.store_message_by_type(MessageInfo {
            message: yellow_child.clone(),
            size_bytes: 136,
            point_estimate: 73, // 18 * 4 + 1
        });
        processing.store_message_by_type(MessageInfo {
            message: blue_valid.clone(),
            size_bytes: 152,
            point_estimate: 160, // 40 * 4
        });
        processing.store_message_by_type(MessageInfo {
            message: blue_large.clone(),
            size_bytes: 4000,    // Large message
            point_estimate: 180, // 45 * 4
        });

        // Fill current batch partially
        let filler_msg = create_test_message(0, 1, None);
        processing.state.current_batch.push(filler_msg);
        processing.state.current_batch_size = 2500;

        processing.optimize_current_batch();

        // Batch size is respected
        assert!(
            processing.state.current_batch_size <= Processing::MAX_BATCH_SIZE,
            "Batch size should not exceed 4096 bytes"
        );

        let batch_signatures: Vec<_> = processing
            .state
            .current_batch
            .iter()
            .map(|m| m.get_signature())
            .collect();

        // If red child was selected, its parent should be in the sent messages
        if batch_signatures.contains(&red_child.get_signature()) {
            assert!(
                processing.state.sent_signatures.contains(&parent_sig),
                "Child message's parent should be in sent messages"
            );
        }

        // Verify that if large message was selected, total size is still under limit
        if batch_signatures.contains(&blue_large.get_signature()) {
            assert!(
                processing.state.current_batch_size <= Processing::MAX_BATCH_SIZE,
                "Large message selection should respect batch size limits"
            );
        }
    }
}
