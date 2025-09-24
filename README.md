# Flux Capacitor

We have bad news. The Flux Capacitor is broken. We need you to fix it.

## What is the Flux Capacitor?

It's an imaginary system with made-up rules, dependencies, and constraints. It is designed to test how you might navigate working on a real system.

At a high level:

```
┌─────────┐    ┌─────────────┐    ┌─────────────┐    ┌──────┐
│ SOURCE  │───▶│ PROCESSING  │───▶│ TRANSMITTER │───▶│ SINK │
└─────────┘    └─────────────┘    └─────────────┘    └──────┘
```

Each of these is an async tokio task, and they are connected via channels.

### Source
Generates imaginary blockchain transactions. There are different types of different sizes (byte lengths).
Transactions may or may not have dependencies on previous transactions (Option<parent_signature>), and each transaction has a different inherent 'value'.

### Processing
This is yours to fill in, you need to read the transactions as they are generated, and pass along transactions in a way that maximizes your 'points'.

### Transmitter
The transmitter reads transactions (only up to 4096 bytes at a time!) every 10ms, and forwards them to the sink.

### Sink
The sink validates the transactions, checks any dependencies, and measures the transaction value.

## Objective and Constraints
Your goal is to get as many points as you can in 10 seconds. The only constraint is that you cannot edit any files in the ./src/constraints directory. Feel free to suggest quality-of-life improvements, or let us know if you think there is a bug, as of this writing this is a brand new technical evaluation.
We encourage you to ask questions. There are no rules other than what is written above, and there are many different ways to approach this problem.# flux-capacitor


### The Core Approach
This algorithm maximizes points within a given time  window while respecting parent-child relationship between the messages and the bandwidth limits. The approach can be summarized as a greedy strategy with lookahead optimization.

```
Three-Tier Strategy:

1. Perfect Triplet Formation:
    Create sequences of 3 different message types to unlock 2x scoring multipliers. Also prioritize child messages which have a recent parent.
2. Greedy Space Filling:
    Pack remaining capacity with (highest value, lowest density) messages
3. Adaptive Re-estimation:
    Update points for messages which did not get selected in the current batch
```

#### Why This Approach is Near-Optimal
1. Mathematically Sound Prioritization
The algorithm correctly identifies that triplet formation is the dominant strategy:

     Normal scoring: base_points × 4 + dependency_bonus
     Triplet scoring: base_points × 8 + dependency_bonus

     Since high-value messages (Blue=20-50 pts, Yellow=10-20 pts) benefit most from this multiplier, prioritizing triplets with high-value messages yields highest returns.

2. Global Optimization Within Constraints
Rather than first-fit triplet formation, the algorithm:

     Tests all 6 possible orderings for each triplet
     Selects globally optimal combination based on total estimated score
     Handles dependency constraints by validating parent availability

     This prevents local optima where a suboptimal ordering blocks better combinations.

3. Intelligent Constraint Handling
The dependency validation is sophisticated:

     Mirrors sink state exactly (100-message history window)
     Predicts 0-point scenarios and avoids them proactively
     Maintains temporal consistency through FIFO expiration

     This prevents the case of selecting high-value messages that will score zero points.

4. Efficient Resource Utilization
The batch packing strategy maximizes value extraction:

     Points-per-byte optimization for remaining space
     Precise size accounting prevents bandwidth waste
     Adaptive thresholds for optimization triggers


#### Shortcomings
1. Limited Lookahead Horizon
The algorithm only considers immediate triplet formation within its buffer without looking ahead to future message arrivals.
The larger the bugger, bigger and better will be the lookahead horizon. But this will also come with time penalty.

2. Batch Boundary Optimization
The 10ms batch boundaries may split optimal message sequences.
