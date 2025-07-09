use crossbeam::channel::unbounded;

pub mod constraints;
pub mod processing;
pub mod types;
pub use constraints::*;
pub use processing::*;
pub use types::*;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    println!("Starting the ~Flux Capacitor~");

    let (source_tx, source_rx) = unbounded::<MessageEnum>();
    let (processing_tx, processing_rx) = unbounded::<MessageEnum>();
    let (sink_tx, sink_rx) = unbounded::<Vec<u8>>();

    let source_handle = tokio::spawn(async move {
        let mut source = Source::new(source_tx);
        println!("Source started...");
        source.start().await;
    });

    let processing_handle = tokio::spawn(async move {
        let mut processing = Processing::new(source_rx, processing_tx);
        println!("Processing started...");
        processing.start().await;
    });

    let transmitter_handle = tokio::spawn(async move {
        let mut transmitter = Transmitter::new(processing_rx, sink_tx);
        println!("Transmitter started...");
        transmitter.start().await;
    });

    let sink_handle = tokio::spawn(async move {
        let mut sink = Sink::new(sink_rx);
        println!("Sink started...");
        sink.start().await;
    });

    tokio::select! {
        _ = source_handle => println!("Source completed"),
        _ = processing_handle => println!("Processing completed"),
        _ = transmitter_handle => println!("Transmitter completed"),
        _ = sink_handle => println!("Sink completed"),
    }
}
