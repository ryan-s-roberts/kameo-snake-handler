use bincode::{Decode, Encode};
use kameo::reply::Reply;
use kameo_child_process::KameoChildProcessMessage;
use kameo_child_process::prelude::SubprocessIpcActorExt;
use kameo_snake_handler::prelude::*;
use kameo_snake_handler::telemetry::build_subscriber_with_otel_and_fmt_async_with_config;
use kameo_snake_handler::telemetry::TelemetryExportConfig;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tokio::time::timeout;
use tracing::{error, info};
use tracing_futures::Instrument;
use rand::{Rng, thread_rng};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use std::env;
use std::fmt::Write as _;
use futures::stream::StreamExt;
use kameo_child_process::error::PythonExecutionError;
use kameo_child_process::callback::{CallbackHandler};
use pyo3::pyfunction;


/// Custom error type for logic operations
#[derive(Debug, Error, Serialize, Deserialize, Clone, Decode, Encode)]
pub enum TestError {
    #[error("Not enough entities for operation (need {needed}, got {got})")]
    NotEnoughEntities { needed: i32, got: i32 },
    #[error("Unknown category: {0}")]
    UnknownCategory(String),
    #[error("Invalid power level: {0}")]
    InvalidPower(String),
}

/// Message types that can be sent to Python subprocess
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub enum TestMessage {
    CalculatePower {
        count: u32,
    },
    CalculateCategoryBonus {
        category_name: String,
        base_power: u32,
    },
    CalculateCompetitionResult {
        attacker_power: u32,
        defender_power: u32,
    },
    CalculateReward {
        currency: u32,
        points: u32,
    },
    CallbackRoundtrip {
        value: u32,
    },
    // New streaming message types
    StreamFibonacci {
        count: u32,
    },
    StreamRandomNumbers {
        count: u32,
        max_value: u32,
    },
    StreamWithDelays {
        count: u32,
        delay_ms: u32,
    },
    StreamWithErrors {
        count: u32,
        error_at: Option<u32>,
    },
    StreamLargeDataset {
        count: u32,
    },
}

impl Default for TestMessage {
    fn default() -> Self {
        Self::CalculatePower { count: 0 }
    }
}

/// Response types from Python subprocess
#[derive(Debug, Serialize, Deserialize, Clone, Decode, Encode)]
pub enum TestResponse {
    Power {
        power: u32,
    },
    CategoryBonus {
        bonus: u32,
    },
    CompetitionResult {
        victory: bool,
    },
    RewardResult {
        total_currency: u32,
        bonus_currency: u32,
    },
    CallbackRoundtripResult {
        value: u32,
    },
    // New streaming response types
    StreamItem {
        index: u32,
        value: u32,
    },
    StreamError {
        index: u32,
        error: String,
    },
    StreamComplete {
        total_items: u32,
    },
}

impl Reply for TestResponse {
    type Ok = Self;
    type Error = TestError;
    type Value = Self;

    fn to_result(self) -> Result<Self::Ok, <Self as Reply>::Error> { Ok(self) }

    fn into_any_err(self) -> Option<Box<dyn kameo::reply::ReplyError>> { None }

    fn into_value(self) -> Self::Value { self }
}

impl KameoChildProcessMessage for TestMessage {
    type Ok = TestResponse;
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct TestCallbackMessage {
    pub value: u32,
}

#[derive(Clone)]
pub struct TestCallbackHandler;

#[async_trait::async_trait]
impl CallbackHandler<TestCallbackMessage> for TestCallbackHandler {
    async fn handle(&self, callback: TestCallbackMessage) -> Result<(), PythonExecutionError> {
        tracing::info!(event = "test_callback", value = callback.value, "TestCallbackHandler received callback");
        Ok(())
    }
}

#[async_trait::async_trait]
impl CallbackHandler<TraderCallbackMessage> for TestCallbackHandler {
    async fn handle(&self, callback: TraderCallbackMessage) -> Result<(), PythonExecutionError> {
        tracing::info!(event = "trader_callback", value = callback.value, "TestCallbackHandler received trader callback");
        Ok(())
    }
}

#[async_trait::async_trait]
impl CallbackHandler<BenchCallback> for TestCallbackHandler {
    async fn handle(&self, callback: BenchCallback) -> Result<(), PythonExecutionError> {
        tracing::info!(event = "bench_callback", id = callback.id, rust_sleep_ms = callback.rust_sleep_ms, "TestCallbackHandler received bench callback");
        Ok(())
    }
}

#[derive(Clone)]
struct CountingCallbackHandler {
    counter: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl CallbackHandler<BenchCallback> for CountingCallbackHandler {
    async fn handle(&self, callback: BenchCallback) -> Result<(), PythonExecutionError> {
        self.counter.fetch_add(1, Ordering::Relaxed);
        tracing::info!(event = "bench_callback", id = callback.id, rust_sleep_ms = callback.rust_sleep_ms, "CountingCallbackHandler received bench callback");
        tokio::time::sleep(Duration::from_millis(callback.rust_sleep_ms)).await;
        Ok(())
    }
}

// --- DSPy Trader Demo Types ---
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub enum TraderMessage {
    OrderDetails { item: String, currency: u32 },
}

#[derive(Debug, Serialize, Deserialize, Clone, Decode, Encode)]
pub enum TraderResponse {
    OrderResult { result: String },
}

impl Reply for TraderResponse {
    type Ok = Self;
    type Error = TestError;
    type Value = Self;

    fn to_result(self) -> Result<Self::Ok, <Self as Reply>::Error> { Ok(self) }

    fn into_any_err(self) -> Option<Box<dyn kameo::reply::ReplyError>> { None }

    fn into_value(self) -> Self::Value { self }
}

impl KameoChildProcessMessage for TraderMessage {
    type Ok = TraderResponse;
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct TraderCallbackMessage {
    pub value: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct BenchMessage {
    pub id: u64,
    pub py_sleep_ms: u64,
    pub rust_sleep_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Decode, Encode)]
pub enum BenchResponse {
    Power { power: u32 },
    CategoryBonus { bonus: u32 },
    CompetitionResult { victory: bool },
    RewardResult { total_currency: u32, bonus_currency: u32 },
    CallbackRoundtripResult { value: u32 },
}

impl Reply for BenchResponse {
    type Ok = Self;
    type Error = TestError;
    type Value = Self;

    fn to_result(self) -> Result<Self::Ok, <Self as Reply>::Error> { Ok(self) }

    fn into_any_err(self) -> Option<Box<dyn kameo::reply::ReplyError>> { None }

    fn into_value(self) -> Self::Value { self }
}

impl KameoChildProcessMessage for BenchMessage {
    type Ok = BenchResponse;
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct BenchCallback {
    pub id: u64,
    pub rust_sleep_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct BenchCallbackReply {
    pub id: u64,
}

impl kameo::reply::Reply for BenchCallbackReply {
    type Ok = Self;
    type Error = ();
    type Value = Self;
    fn to_result(self) -> Result<Self::Ok, Self::Error> { Ok(self) }
    fn into_any_err(self) -> Option<Box<dyn kameo::reply::ReplyError>> { None }
    fn into_value(self) -> Self::Value { self }
}

const POOL_SIZE: usize = 4;

#[tracing::instrument]
async fn run_sync_tests(python_path: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    // Create a root span for the sync test to ensure proper trace context
    let test_span = tracing::info_span!("sync-test-run");
    let _test_guard = test_span.enter();
    
    let sync_config = PythonConfig {
        python_path: python_path.clone(),
        module_name: "logic".to_string(),
        function_name: "handle_message".to_string(),
        env_vars: vec![],
        is_async: false,
        module_path: "crates/kameo-snake-testing/python/logic.py".to_string(),
    };
    tracing::trace!(
        event = "test_spawn",
        step = "before_spawn",
        "About to spawn Python child process"
    );
    let sync_pool = PythonChildProcessBuilder::<TestMessage, TestCallbackMessage>::new(sync_config)
        .with_callback_handler(TestCallbackHandler)
        .spawn_pool(POOL_SIZE, None)
        .await?;
    let sync_ref = sync_pool.get_actor();
    tracing::trace!(
        event = "test_spawn",
        step = "after_spawn",
        "Returned from spawn, about to send first .ask()"
    );
    tracing::trace!(
        event = "test_send",
        step = "before_ask",
        "About to send first sync .ask() to child actor"
    );
    let resp = sync_ref
        .ask(TestMessage::CalculatePower { count: 100 })
        .await;
    tracing::trace!(
        event = "test_send",
        step = "after_ask",
        ?resp,
        "Received response from first sync .ask() to child actor"
    );
    assert!(
        matches!(resp, Ok(TestResponse::Power { .. })),
        "SYNC Test 1 failed: got {:?}",
        resp
    );

    // Test 2: Invalid message (unknown category)
    let resp = sync_ref
        .ask(TestMessage::CalculateCategoryBonus {
            category_name: "UnknownCategory".to_string(),
            base_power: 0,
        })
        .await;
    assert!(resp.is_err(), "SYNC Test 2 should error, got {:?}", resp);

    // Test 3: Edge case - zero count
    let resp = sync_ref.ask(TestMessage::CalculatePower { count: 0 }).await;
    assert!(resp.is_err(), "SYNC Test 3 should error, got {:?}", resp);

    // Test 4: Edge case - massive number
    let resp = sync_ref
        .ask(TestMessage::CalculatePower { count: u32::MAX })
        .await;
    assert!(resp.is_err(), "SYNC Test 4 should error, got {:?}", resp);

    // Test 5: Competition result test
    let resp = sync_ref
        .ask(TestMessage::CalculateCompetitionResult {
            attacker_power: 1000,
            defender_power: 500,
        })
        .await;
    assert!(
        matches!(resp, Ok(TestResponse::CompetitionResult { .. })),
        "SYNC Test 5 failed: got {:?}",
        resp
    );

    // Test 6: Reward calculation
    let resp = sync_ref
        .ask(TestMessage::CalculateReward {
            currency: 100,
            points: 5,
        })
        .await;
    assert!(
        matches!(resp, Ok(TestResponse::RewardResult { .. })),
        "SYNC Test 6 failed: got {:?}",
        resp
    );
    Ok(())
}

async fn run_async_tests(python_path: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    
    // Create a root span for the async test to ensure proper trace context
    let test_span = tracing::info_span!("async-test-run");
    let _test_guard = test_span.enter();
    
    // Debug: Verify the test span is active
    tracing::debug!(
        event = "test_span_debug",
        span_id = ?test_span.id(),
        "Test span created and entered"
    );
    
    let async_config = PythonConfig {
        python_path: python_path.clone(),
        module_name: "logic_async".to_string(),
        function_name: "handle_message_async".to_string(),
        env_vars: vec![],
        is_async: true,
        module_path: "crates/kameo-snake-testing/python/logic_async.py".to_string(),
    };
    let async_pool = PythonChildProcessBuilder::<TestMessage, TestCallbackMessage>::new(async_config)
        .with_callback_handler(TestCallbackHandler)
        .spawn_pool(POOL_SIZE, None)
        .await?;
    let async_ref = async_pool.get_actor();

    // Test 1: Valid message
    let resp = async_ref
        .ask(TestMessage::CalculatePower { count: 100 })
        .await;
    assert!(
        matches!(resp, Ok(TestResponse::Power { .. })),
        "ASYNC Test 1 failed: got {:?}",
        resp
    );

    // Test 2: Invalid message (unknown category)
    let resp = async_ref
        .ask(TestMessage::CalculateCategoryBonus {
            category_name: "UnknownCategory".to_string(),
            base_power: 0,
        })
        .await;
    assert!(resp.is_err(), "ASYNC Test 2 should error, got {:?}", resp);

    // Test 3: Edge case - zero count
    let resp = async_ref
        .ask(TestMessage::CalculatePower { count: 0 })
        .await;
    assert!(resp.is_err(), "ASYNC Test 3 should error, got {:?}", resp);

    // Test 4: Edge case - massive number
    let resp = async_ref
        .ask(TestMessage::CalculatePower { count: u32::MAX })
        .await;
    assert!(resp.is_err(), "ASYNC Test 4 should error, got {:?}", resp);

    // Test 5: Competition result test
    let resp = async_ref
        .ask(TestMessage::CalculateCompetitionResult {
            attacker_power: 1000,
            defender_power: 500,
        })
        .await;
    assert!(
        matches!(resp, Ok(TestResponse::CompetitionResult { .. })),
        "ASYNC Test 5 failed: got {:?}",
        resp
    );

    // Test 6: Reward calculation
    let resp = async_ref
        .ask(TestMessage::CalculateReward {
            currency: 100,
            points: 5,
        })
        .await;
    assert!(
        matches!(resp, Ok(TestResponse::RewardResult { .. })),
        "ASYNC Test 6 failed: got {:?}",
        resp
    );

    // Test: Callback roundtrip
    let resp = async_ref
        .ask(TestMessage::CallbackRoundtrip { value: 42 })
        .await;
    match resp {
        Ok(TestResponse::CallbackRoundtripResult { value }) => {
            assert_eq!(value, 43, "Callback roundtrip value should be incremented");
        }
        Ok(other) => panic!("ASYNC callback roundtrip: Unexpected response: {:?}", other),
        Err(e) => panic!("ASYNC callback roundtrip failed: {:?}", e),
    }

    // Test 7: Rapid fire messages (stress test)
    let mut handles = Vec::new();
    for i in 0..10 {
        if i == 0 {
            continue;
        } // skip count=0, which is always an error
        let ref_clone = async_ref.clone();
        handles.push(tokio::spawn(async move {
            ref_clone
                .ask(TestMessage::CalculatePower { count: i * 100 })
                .await
        }));
    }
    let results = futures::future::join_all(handles).await;
    for (i, res) in results.into_iter().enumerate() {
        let resp = res.expect("Task panicked");
        assert!(
            matches!(resp, Ok(TestResponse::Power { .. })),
            "Rapid fire {} failed: {:?}",
            i + 1,
            resp
        );
    }
    Ok(())
}

async fn run_streaming_tests(python_path: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    info!("==== STREAMING TESTS ====");
    
    let test_span = tracing::info_span!("streaming-test-run");
    let _test_guard = test_span.enter();
    
    let streaming_config = PythonConfig {
        python_path: python_path.clone(),
        module_name: "logic_streaming".to_string(),
        function_name: "handle_message_streaming".to_string(),
        env_vars: vec![],
        is_async: true,
        module_path: "crates/kameo-snake-testing/python/logic_streaming.py".to_string(),
    };
    let streaming_pool = PythonChildProcessBuilder::<TestMessage, TestCallbackMessage>::new(streaming_config)
        .with_callback_handler(TestCallbackHandler)
        .spawn_pool(POOL_SIZE, None)
        .await?;
    let streaming_ref = streaming_pool.get_actor();

    // Test 1: Fibonacci stream
    info!("Test 1: Fibonacci stream");
    let mut stream = streaming_ref.send_stream(TestMessage::StreamFibonacci { count: 10 })
        .await?;
    
    let mut items = Vec::new();
    let stream_future = async {
        while let Some(result) = stream.next().await {
            match result {
                Ok(TestResponse::StreamItem { index, value }) => {
                    items.push((index, value));
                    info!("Received Fibonacci item: index={}, value={}", index, value);
                }
                Ok(TestResponse::StreamError { index, error }) => {
                    error!("Received stream error: index={}, error={}", index, error);
                    break;
                }
                Ok(other) => {
                    info!("Received unexpected response: {:?}", other);
                }
                Err(e) => {
                    error!("Stream error: {:?}", e);
                    break;
                }
            }
        }
    };
    
    // Timeout the stream processing
    match timeout(Duration::from_secs(30), stream_future).await {
        Ok(_) => {},
        Err(_) => {
            error!("Fibonacci stream timed out after 30 seconds");
        }
    }
    
    assert_eq!(items.len(), 10, "Should receive 10 Fibonacci numbers");
    assert_eq!(items[0].1, 0, "First Fibonacci number should be 0");
    assert_eq!(items[1].1, 1, "Second Fibonacci number should be 1");
    assert_eq!(items[2].1, 1, "Third Fibonacci number should be 1");
    assert_eq!(items[3].1, 2, "Fourth Fibonacci number should be 2");

    // Test 2: Random numbers stream
    info!("Test 2: Random numbers stream");
    let mut stream = streaming_ref.send_stream(TestMessage::StreamRandomNumbers { count: 5, max_value: 100 })
        .await?;
    
    let mut items = Vec::new();
    let stream_future = async {
        while let Some(result) = stream.next().await {
            match result {
                Ok(TestResponse::StreamItem { index, value }) => {
                    items.push((index, value));
                    info!("Received random number: index={}, value={}", index, value);
                }
                Ok(TestResponse::StreamError { index, error }) => {
                    error!("Received stream error: index={}, error={}", index, error);
                    break;
                }
                Ok(other) => {
                    info!("Received unexpected response: {:?}", other);
                }
                Err(e) => {
                    error!("Stream error: {:?}", e);
                    break;
                }
            }
        }
    };
    
    // Timeout the stream processing
    match timeout(Duration::from_secs(30), stream_future).await {
        Ok(_) => {},
        Err(_) => {
            error!("Random numbers stream timed out after 30 seconds");
        }
    }
    
    assert_eq!(items.len(), 5, "Should receive 5 random numbers");
    for (_index, value) in items {
        assert!((1..=100).contains(&value), "Random number should be between 1 and 100");
    }

    // Test 3: Stream with delays
    info!("Test 3: Stream with delays");
    let start = std::time::Instant::now();
    let mut stream = streaming_ref.send_stream(TestMessage::StreamWithDelays { count: 3, delay_ms: 50 })
        .await?;
    
    let mut items = Vec::new();
    let stream_future = async {
        while let Some(result) = stream.next().await {
            match result {
                Ok(TestResponse::StreamItem { index, value }) => {
                    items.push((index, value));
                    info!("Received delayed item: index={}, value={}", index, value);
                }
                Ok(TestResponse::StreamError { index, error }) => {
                    error!("Received stream error: index={}, error={}", index, error);
                    break;
                }
                Ok(other) => {
                    info!("Received unexpected response: {:?}", other);
                }
                Err(e) => {
                    error!("Stream error: {:?}", e);
                    break;
                }
            }
        }
    };
    
    // Timeout the stream processing
    match timeout(Duration::from_secs(30), stream_future).await {
        Ok(_) => {},
        Err(_) => {
            error!("Delayed stream timed out after 30 seconds");
        }
    }
    
    let elapsed = start.elapsed();
    assert_eq!(items.len(), 3, "Should receive 3 delayed items");
    assert!(elapsed >= std::time::Duration::from_millis(100), "Should have delays");

    // Test 4: Stream with errors
    info!("Test 4: Stream with errors");
    let mut stream = streaming_ref.send_stream(TestMessage::StreamWithErrors { count: 5, error_at: Some(2) })
        .await?;
    
    let mut items = Vec::new();
    let mut error_received = false;
    let stream_future = async {
        while let Some(result) = stream.next().await {
            match result {
                Ok(TestResponse::StreamItem { index, value }) => {
                    items.push((index, value));
                    info!("Received item before error: index={}, value={}", index, value);
                }
                Ok(TestResponse::StreamError { index, error }) => {
                    error_received = true;
                    info!("Received expected error: index={}, error={}", index, error);
                    break;
                }
                Ok(other) => {
                    info!("Received unexpected response: {:?}", other);
                }
                Err(e) => {
                    error!("Stream error: {:?}", e);
                    break;
                }
            }
        }
    };
    
    // Timeout the stream processing
    match timeout(Duration::from_secs(30), stream_future).await {
        Ok(_) => {},
        Err(_) => {
            error!("Error stream timed out after 30 seconds");
        }
    }
    
    assert!(error_received, "Should receive an error");
    assert_eq!(items.len(), 2, "Should receive 2 items before error");

    // Test 5: Large dataset stream
    info!("Test 5: Large dataset stream");
    let mut stream = streaming_ref.send_stream(TestMessage::StreamLargeDataset { count: 10 })
        .await?;
    
    let mut items = Vec::new();
    let stream_future = async {
        while let Some(result) = stream.next().await {
            match result {
                Ok(TestResponse::StreamItem { index, value }) => {
                    items.push((index, value));
                    info!("Received large dataset item: index={}, value={}", index, value);
                }
                Ok(TestResponse::StreamError { index, error }) => {
                    error!("Received stream error: index={}, error={}", index, error);
                    break;
                }
                Ok(other) => {
                    info!("Received unexpected response: {:?}", other);
                }
                Err(e) => {
                    error!("Stream error: {:?}", e);
                    break;
                }
            }
        }
    };
    
    // Timeout the stream processing
    match timeout(Duration::from_secs(30), stream_future).await {
        Ok(_) => {},
        Err(_) => {
            error!("Large dataset stream timed out after 30 seconds");
        }
    }
    
    assert_eq!(items.len(), 10, "Should receive 10 large dataset items");

    Ok(())
}

async fn run_streaming_throughput_test(python_path: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    info!("==== STREAMING THROUGHPUT TEST ====");
    
    const N: usize = 1000;
    const STREAM_SIZE: u32 = 10;
    
    let streaming_config = PythonConfig {
        python_path: python_path.clone(),
        module_name: "logic_streaming".to_string(),
        function_name: "handle_message_streaming".to_string(),
        env_vars: vec![],
        is_async: true,
        module_path: "crates/kameo-snake-testing/python/logic_streaming.py".to_string(),
    };
    let streaming_pool = PythonChildProcessBuilder::<TestMessage, TestCallbackMessage>::new(streaming_config)
        .with_callback_handler(TestCallbackHandler)
        .spawn_pool(100, None)
        .await?;
    
    let start = Instant::now();
    let mut handles = futures::stream::FuturesUnordered::new();
    let mut total_items = 0;
    
    for _i in 0..N {
        let streaming_ref = streaming_pool.get_actor();
        handles.push(tokio::spawn(async move {
            let msg = TestMessage::StreamRandomNumbers { 
                count: STREAM_SIZE, 
                max_value: 1000 
            };
            streaming_ref.send_stream(msg).await
        }));
    }
    
    while let Some(res) = handles.next().await {
        match res {
            Ok(Ok(mut stream)) => {
                // Process the stream
                while let Some(stream_result) = stream.next().await {
                    match stream_result {
                        Ok(TestResponse::StreamItem { .. }) => {
                            total_items += 1;
                        }
                        Ok(TestResponse::StreamError { .. }) => {
                            // Count errors as items for throughput calculation
                            total_items += 1;
                        }
                        Ok(other) => {
                            tracing::warn!(?other, "Unexpected streaming response");
                        }
                        Err(e) => {
                            tracing::error!(?e, "Stream error");
                        }
                    }
                }
            }
            Ok(Err(e)) => {
                tracing::error!(?e, "Streaming error");
            }
            Err(e) => {
                tracing::error!(?e, "Task join error");
            }
        }
    }
    
    let elapsed = start.elapsed();
    let throughput = total_items as f64 / elapsed.as_secs_f64();
    
    let mut table = String::new();
    writeln!(table, "\n┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓").unwrap();
    writeln!(table,   "┃        🐍  STREAMING THROUGHPUT STATS  🦀        ┃").unwrap();
    writeln!(table,   "┣━━━━━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━┫").unwrap();
    writeln!(table,   "┃ Metric                    ┃ Value                 ┃").unwrap();
    writeln!(table,   "┣━━━━━━━━━━━━━━━━━━━━━━━━━━━╋━━━━━━━━━━━━━━━━━━━━━━━┫").unwrap();
    writeln!(table,   "┃ Total streams             ┃ {:>9}              ┃", N).unwrap();
    writeln!(table,   "┃ Total items               ┃ {:>9}              ┃", total_items).unwrap();
    writeln!(table,   "┃ Elapsed                   ┃ {:>9.3} s           ┃", elapsed.as_secs_f64()).unwrap();
    writeln!(table,   "┃ Throughput                ┃ {:>9.1} items/sec   ┃", throughput).unwrap();
    writeln!(table,   "┗━━━━━━━━━━━━━━━━━━━━━━━━━━━┻━━━━━━━━━━━━━━━━━━━━━━━┛").unwrap();
    
    if throughput > 1000.0 {
        println!("{}\n✅ Streaming throughput is excellent!", table);
    } else {
        println!("{}\n⚠️  Streaming throughput is below target!", table);
    }
    
    Ok(())
}

async fn run_streaming_error_handling_test(python_path: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    info!("==== STREAMING ERROR HANDLING TEST ====");
    
    let streaming_config = PythonConfig {
        python_path: python_path.clone(),
        module_name: "logic_streaming".to_string(),
        function_name: "handle_message_streaming".to_string(),
        env_vars: vec![],
        is_async: true,
        module_path: "crates/kameo-snake-testing/python/logic_streaming.py".to_string(),
    };
    let streaming_pool = PythonChildProcessBuilder::<TestMessage, TestCallbackMessage>::new(streaming_config)
        .with_callback_handler(TestCallbackHandler)
        .spawn_pool(POOL_SIZE, None)
        .await?;
    let streaming_ref = streaming_pool.get_actor();

    // Test 1: Error at first item
    info!("Test 1: Error at first item");
    let stream_result = streaming_ref.send_stream(TestMessage::StreamWithErrors { count: 5, error_at: Some(0) })
        .await;
    match stream_result {
        Ok(mut stream) => {
            let first_item = stream.next().await;
            assert!(
                matches!(first_item, Some(Ok(TestResponse::StreamError { .. }))),
                "Error handling Test 1 failed: got {:?}",
                first_item
            );
        }
        Err(e) => {
            panic!("Error handling Test 1 failed: got error {:?}", e);
        }
    }

    // Test 2: Error at last item
    info!("Test 2: Error at last item");
    let stream_result = streaming_ref.send_stream(TestMessage::StreamWithErrors { count: 5, error_at: Some(4) })
        .await;
    match stream_result {
        Ok(mut stream) => {
            let first_item = stream.next().await;
            assert!(
                matches!(first_item, Some(Ok(TestResponse::StreamItem { .. }))),
                "Error handling Test 2 failed: got {:?}",
                first_item
            );
        }
        Err(e) => {
            panic!("Error handling Test 2 failed: got error {:?}", e);
        }
    }

    // Test 3: No error (should complete successfully)
    info!("Test 3: No error (should complete successfully)");
    let stream_result = streaming_ref.send_stream(TestMessage::StreamWithErrors { count: 3, error_at: None })
        .await;
    match stream_result {
        Ok(mut stream) => {
            let first_item = stream.next().await;
            assert!(
                matches!(first_item, Some(Ok(TestResponse::StreamItem { .. }))),
                "Error handling Test 3 failed: got {:?}",
                first_item
            );
        }
        Err(e) => {
            panic!("Error handling Test 3 failed: got error {:?}", e);
        }
    }

    Ok(())
}

async fn run_invalid_config_tests(
    python_path: Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Test 8: Invalid module test
    info!("Test 8: Invalid module test");
    let invalid_module_config = PythonConfig {
        python_path: python_path.clone(),
        module_name: "non_existent_module".to_string(),
        function_name: "handle_message".to_string(),
        env_vars: vec![],
        is_async: false,
        module_path: "crates/kameo-snake-testing/python/non_existent_module.py".to_string(),
    };

    let spawn_result = timeout(
        Duration::from_secs(31),
        PythonChildProcessBuilder::<TestMessage, TestCallbackMessage>::new(invalid_module_config)
            .spawn_pool(POOL_SIZE, None),
    )
    .await;
    match spawn_result {
        Ok(_actor_ref) => panic!("Spawning with invalid module should fail"),
        Err(e) => info!("Received expected error on spawn: {}", e),
    }

    // Test 9: Invalid function test
    info!("Test 9: Invalid function test");
    let invalid_function_config = PythonConfig {
        python_path: python_path.clone(),
        module_name: "logic".to_string(),
        function_name: "non_existent_function".to_string(),
        env_vars: vec![],
        is_async: false,
        module_path: "crates/kameo-snake-testing/python/logic.py".to_string(),
    };
    let spawn_result = timeout(
        Duration::from_secs(31),
        PythonChildProcessBuilder::<TestMessage, TestCallbackMessage>::new(invalid_function_config)
            .spawn_pool(POOL_SIZE, None),
    )
    .await;
    match spawn_result {
        Ok(_actor_ref) => panic!("Spawning with invalid function should fail"),
        Err(e) => info!("Received expected error on spawn: {}", e),
    }

    // Test 10: Invalid path test
    info!("Test 10: Invalid path test");
    let invalid_path_config = PythonConfig {
        python_path: python_path.clone(),
        module_name: "logic".to_string(),
        function_name: "handle_message".to_string(),
        env_vars: vec![],
        is_async: false,
        module_path: "crates/kameo-snake-testing/python/logic.py".to_string(),
    };
    let spawn_result = timeout(
        Duration::from_secs(32),
        PythonChildProcessBuilder::<TestMessage, TestCallbackMessage>::new(invalid_path_config)
            .spawn_pool(POOL_SIZE, None),
    )
    .await;
    match spawn_result {
        Ok(_actor_ref) => panic!("Spawning with invalid path should fail"),
        Err(e) => info!("Received expected error on spawn: {}", e),
    }

    Ok(())
}

#[tracing::instrument]
async fn run_trader_demo(python_path: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let trader_config = PythonConfig {
        python_path: python_path.clone(),
        module_name: "dspy_trader".to_string(),
        function_name: "handle_message".to_string(),
        env_vars: vec![],
        is_async: true,
        module_path: "crates/kameo-snake-testing/python/dspy_trader.py".to_string(),
    };
    let trader_pool = PythonChildProcessBuilder::<TraderMessage, TraderCallbackMessage>::new(trader_config)
        .with_callback_handler(TestCallbackHandler)
        .spawn_pool(POOL_SIZE, None)
        .await?;
    let trader_ref = trader_pool.get_actor();
    let resp = trader_ref
        .ask(TraderMessage::OrderDetails {
            item: "widget".to_string(),
            currency: 42,
        })
        .await;
    tracing::info!(?resp, "Trader demo response");
    assert!(
        matches!(resp, Ok(TraderResponse::OrderResult { .. })),
        "Trader demo failed: got {:?}",
        resp
    );
    Ok(())
}

async fn run_bench_throughput_test(python_path: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    const N: usize = 10000;
    const MAX_SLEEP_MS: u64 = 10;
    let mut rng = thread_rng();
    let bench_config = PythonConfig {
        python_path: python_path.clone(),
        module_name: "bench_async".to_string(),
        function_name: "handle_bench_message".to_string(),
        env_vars: vec![],
        is_async: true,
        module_path: "crates/kameo-snake-testing/python/bench_async.py".to_string(),
    };
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_handler = CountingCallbackHandler { counter: callback_count.clone() };
    let bench_pool = PythonChildProcessBuilder::<BenchMessage, BenchCallback>::new(bench_config)
        .with_callback_handler(callback_handler)
        .spawn_pool(1000, None)
        .await?;
    let start = Instant::now();
    let in_flight = Arc::new(AtomicUsize::new(0));
    let max_concurrency = Arc::new(AtomicUsize::new(0));
    let mut handles = futures::stream::FuturesUnordered::new();
    let mut latencies = Vec::with_capacity(N);
    for i in 0..N {
        let py_sleep_ms = rng.gen_range(10..=MAX_SLEEP_MS);
        let rust_sleep_ms = rng.gen_range(10..=MAX_SLEEP_MS);
        let msg = BenchMessage { id: i as u64, py_sleep_ms, rust_sleep_ms };
        let bench_ref = bench_pool.get_actor();
        let in_flight = in_flight.clone();
        let max_concurrency = max_concurrency.clone();
        handles.push(tokio::spawn(async move {
            let t0 = Instant::now();
            let cur = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            max_concurrency.fetch_max(cur, Ordering::SeqCst);
            let resp = bench_ref.ask(msg).await;
            in_flight.fetch_sub(1, Ordering::SeqCst);
            let latency = t0.elapsed();
            (resp, latency)
        }));
    }
    
    // Add timeout to the entire processing loop
    let process_future = async {
        while let Some(res) = handles.next().await {
            tracing::info!(?res, "Bench result");
            match res {
                Ok((Ok(_), latency)) => latencies.push(latency),
                Ok((Err(e), _)) => eprintln!("Bench error: {e}"),
                Err(e) => eprintln!("Task join error: {e}"),
            }
        }
    };
    
    // Timeout the entire processing after 60 seconds
    match timeout(Duration::from_secs(60), process_future).await {
        Ok(_) => {},
        Err(_) => {
            eprintln!("Bench test timed out after 60 seconds");
            // Continue with partial results
        }
    }
    
    let elapsed = start.elapsed();
    let callbacks = callback_count.load(Ordering::SeqCst);
    let max_conc = max_concurrency.load(Ordering::SeqCst);
    let total = latencies.len();
    let mean = latencies.iter().map(|d| d.as_secs_f64()).sum::<f64>() / total as f64;
    let min = latencies.iter().map(|d| d.as_secs_f64()).fold(f64::INFINITY, f64::min);
    let max = latencies.iter().map(|d| d.as_secs_f64()).fold(0.0, f64::max);
    // Histogram buckets: <20ms, 20-40ms, 40-60ms, 60-80ms, 80-100ms, >=100ms
    let mut buckets = [0usize; 6];
    for d in &latencies {
        let ms = d.as_secs_f64() * 1000.0;
        let idx = if ms < 20.0 {
            0
        } else if ms < 40.0 {
            1
        } else if ms < 60.0 {
            2
        } else if ms < 80.0 {
            3
        } else if ms < 100.0 {
            4
        } else {
            5
        };
        buckets[idx] += 1;
    }
    let mut table = String::new();
    writeln!(table, "\n┏━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓").unwrap();
    writeln!(table,   "┃        🐍  PYTHON BENCHMARK STATS  🦀            ┃").unwrap();
    writeln!(table,   "┣━━━━━━━━━━━━━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━━━━┫").unwrap();
    writeln!(table,   "┃ Metric                    ┃ Value                 ┃").unwrap();
    writeln!(table,   "┣━━━━━━━━━━━━━━━━━━━━━━━━━━━╋━━━━━━━━━━━━━━━━━━━━━━━┫").unwrap();
    writeln!(table,   "┃ Total ops                 ┃ {:>9}              ┃", total).unwrap();
    writeln!(table,   "┃ Elapsed                   ┃ {:>9.3} s           ┃", elapsed.as_secs_f64()).unwrap();
    let throughput = total as f64 / elapsed.as_secs_f64();
    writeln!(table,   "┃ Throughput                ┃ {:>9.1} ops/sec     ┃", throughput).unwrap();
    writeln!(table,   "┃ Max concurrency           ┃ {:>9}              ┃", max_conc).unwrap();
    writeln!(table,   "┃ Latency min               ┃ {:>9.3} ms         ┃", min*1000.0).unwrap();
    writeln!(table,   "┃ Latency mean              ┃ {:>9.3} ms         ┃", mean*1000.0).unwrap();
    writeln!(table,   "┃ Latency max               ┃ {:>9.3} ms         ┃", max*1000.0).unwrap();
    writeln!(table,   "┃ Total callbacks           ┃ {:>9}              ┃", callbacks).unwrap();
    let callback_throughput = if elapsed.as_secs_f64() > 0.0 { callbacks as f64 / elapsed.as_secs_f64() } else { 0.0 };
    writeln!(table,   "┃ Callback throughput       ┃ {:>9.1} cb/sec      ┃", callback_throughput).unwrap();
    writeln!(table,   "┣━━━━━━━━━━━━━━━━━━━━━━━━━━━╋━━━━━━━━━━━━━━━━━━━━━━━┫").unwrap();
    writeln!(table,   "┃ Latency histogram         ┃                       ┃").unwrap();
    writeln!(table,   "┃   < 20 ms                 ┃ {:>5}                ┃", buckets[0]).unwrap();
    writeln!(table,   "┃   20–40 ms                ┃ {:>5}                ┃", buckets[1]).unwrap();
    writeln!(table,   "┃   40–60 ms                ┃ {:>5}                ┃", buckets[2]).unwrap();
    writeln!(table,   "┃   60–80 ms                ┃ {:>5}                ┃", buckets[3]).unwrap();
    writeln!(table,   "┃   80–100 ms               ┃ {:>5}                ┃", buckets[4]).unwrap();
    writeln!(table,   "┃   >= 100 ms               ┃ {:>5}                ┃", buckets[5]).unwrap();
    writeln!(table,   "┗━━━━━━━━━━━━━━━━━━━━━━━━━━━┻━━━━━━━━━━━━━━━━━━━━━━━┛").unwrap();
    if throughput > 500.0 {
        println!("{}\n✅ Throughput is excellent!", table);
    } else {
        println!("{}\n⚠️  Throughput is below target!", table);
    }
    Ok(())
}



kameo_snake_handler::setup_python_subprocess_system! {
    actor = (TestMessage, TestCallbackMessage),
    actor = (TraderMessage, TraderCallbackMessage),
    actor = (BenchMessage, BenchCallback),
    child_init = {{
        kameo_child_process::RuntimeConfig {
            flavor: kameo_child_process::RuntimeFlavor::MultiThread,
            worker_threads: Some(8),
        }
    }},
    parent_init = {
        // Create parent runtime
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("test-main")
            .enable_all()
            .build()?;

        // Initialize the callback handle before any tests run

        let args: Vec<String> = env::args().collect();
        let run_all = args.len() == 1;
        let run_sync = run_all || args.iter().any(|a| a == "sync");
        let run_async = run_all || args.iter().any(|a| a == "async");
        let run_trader = run_all || args.iter().any(|a| a == "trader");
        let run_bench = run_all || args.iter().any(|a| a == "bench");
        let run_module = args.iter().any(|a| a == "module");
        let run_streaming = run_all || args.iter().any(|a| a == "streaming");
        let run_streaming_throughput = run_all || args.iter().any(|a| a == "streaming-throughput");
        let run_streaming_errors = run_all || args.iter().any(|a| a == "streaming-errors");
        if args.iter().any(|a| a == "--help" || a == "-h") {
            println!("Usage: kameo-snake-testing [sync] [async] [trader] [bench] [module] [streaming] [streaming-throughput] [streaming-errors]");
            println!("  If no args, runs all tests.");
            return Ok(());
        }

        runtime.block_on(async {
            // Use OpenTelemetry + fmt subscriber, respects RUST_LOG/env_filter
            let (subscriber, _guard) = build_subscriber_with_otel_and_fmt_async_with_config(
                TelemetryExportConfig {
                    otlp_enabled: true,
                    stdout_enabled: true,
                    metrics_enabled: true,
                }
            ).await;
            tracing::subscriber::set_global_default(subscriber).expect("set global");
            tracing::info!("Parent runtime initialized");

            let python_path = std::env::current_dir()?
                .join("crates")
                .join("kameo-snake-testing")
                .join("python");
            let site_packages = "crates/kameo-snake-testing/python/venv/lib/python3.13/site-packages";
            let python_path_vec = vec![
                site_packages.to_string(),
                python_path.to_string_lossy().to_string(),
            ];
            if run_sync {
                run_sync_tests(python_path_vec.clone()).await?;
            }
            if run_async {
                run_async_tests(python_path_vec.clone()).await?;
            }
            if run_trader {
                run_trader_demo(python_path_vec.clone()).await?;
            }
            if run_bench {
                run_bench_throughput_test(python_path_vec.clone()).await?;
            }
            if run_module {
                run_invalid_config_tests(python_path_vec.clone()).await?;
            }
            if run_streaming {
                run_streaming_tests(python_path_vec.clone()).await?;
            }
            if run_streaming_throughput {
                run_streaming_throughput_test(python_path_vec.clone()).await?;
            }
            if run_streaming_errors {
                run_streaming_error_handling_test(python_path_vec.clone()).await?;
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        })?
    }
}
