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
We encourage you to ask questions. There are no rules other than what is written above, and there are many different ways to approach this problem.