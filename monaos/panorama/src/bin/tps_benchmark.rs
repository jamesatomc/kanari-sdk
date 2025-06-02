// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use clap::{Parser, ValueEnum};
use log::{error, info, warn};
use once_cell::sync::OnceCell;
use rand::Rng;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

// Import panorama components
use mona_blockchain::block::Transaction;
use mona_blockchain::blockchain::{BALANCES, save_blockchain};
use mona_types::address::Address;
use panorama::simulation::add_pending_transaction;

// Global logger initialization
static LOGGER: OnceCell<()> = OnceCell::new();

fn init_logger() {
    LOGGER.get_or_init(|| {
        env_logger::builder()
            .filter_level(log::LevelFilter::Info)
            .init();
    });
}

// Transaction types for more realistic testing
#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
enum TransactionType {
    /// Simple transfer between accounts
    Transfer,
    /// Complex transaction with data payload
    Complex,
    /// Mix of different transaction types
    Mixed,
}

// Network condition simulation options
#[derive(Debug, Clone)]
struct NetworkConditions {
    // Artificial latency in ms
    latency_ms: u64,
    // Probability of transaction failure (0.0-1.0)
    failure_rate: f64,
}

// Create mock transactions without real signing
fn create_mock_transaction(
    sender: &Address,
    receiver: &Address,
    amount: u64,
    tx_type: TransactionType,
    complexity_factor: usize,
) -> Transaction {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Generate a transaction ID
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let transaction_id = format!(
        "mock_tx_{}_{}_{}",
        sender.to_hex_literal(),
        receiver.to_hex_literal(),
        now
    );

    // Create signature with varying complexity
    let mut dummy_signature = vec![0u8; 64]; // Base signature size

    // For complex transactions, add a data payload based on complexity factor
    if tx_type == TransactionType::Complex
        || (tx_type == TransactionType::Mixed && rand::thread_rng().gen_bool(0.5))
    {
        // Add extra data to simulate more complex transaction
        for i in 0..complexity_factor {
            dummy_signature.push((i % 255) as u8);
        }
    }

    Transaction {
        transaction_id,
        sender: sender.clone(),
        receiver: receiver.clone(),
        amount,
        timestamp: now,
        gas_fee: 10 + (complexity_factor as u64 / 10), // Gas fee scales with complexity
        signature: dummy_signature,
        data: None, // No real data payload
    }
}

// Test environment setup with multiple accounts option
fn setup_test_environment(account_count: usize) -> Vec<Address> {
    // Initialize logger only once
    init_logger();

    let mut addresses = Vec::with_capacity(account_count);

    // Create main test address
    let main_address = Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678")
        .expect("Failed to create test address");
    addresses.push(main_address.clone());

    // Initialize test balances with large amount
    {
        let mut balances = BALANCES.lock().unwrap();
        balances.insert(main_address.to_hex_literal(), 10_000_000_000_000); // 10M test tokens

        // Create additional accounts if requested
        if account_count > 1 {
            for i in 1..account_count {
                let addr_hex = format!("0x{:064x}", i);
                if let Ok(addr) = Address::from_hex_literal(&addr_hex) {
                    addresses.push(addr.clone());
                    balances.insert(addr.to_hex_literal(), 1_000_000_000); // 1B tokens for each account
                }
            }
        }
    }

    addresses
}

// Generate batch of mock transactions with configurable parameters
fn generate_batch_transactions(
    addresses: &[Address],
    count: usize,
    tx_type: TransactionType,
    complexity_factor: usize,
) -> Vec<Transaction> {
    let mut transactions = Vec::with_capacity(count);
    let mut rng = rand::thread_rng();

    // Pre-generate receiver addresses
    let mut receiver_addresses = Vec::with_capacity(count);
    let sender_prefix = &addresses[0].to_hex_literal()[2..34]; // Use prefix but trim "0x"

    // Prepare all receiver addresses in advance
    for i in 0..count {
        // Use existing addresses sometimes if we have multiple accounts
        if addresses.len() > 1 && rng.gen_bool(0.3) {
            let idx = rng.gen_range(0..addresses.len());
            receiver_addresses.push(addresses[idx].clone());
        } else {
            // Generate a new address
            let receiver_hex = format!("0x{}{:08x}", sender_prefix, i + 1);
            match Address::from_hex_literal(&receiver_hex) {
                Ok(addr) => receiver_addresses.push(addr),
                Err(e) => {
                    warn!("Failed to create receiver address: {}", e);
                    continue;
                }
            }
        }
    }

    info!("Created {} receiver addresses", receiver_addresses.len());

    // Generate mock transactions in batches
    const BATCH_SIZE: usize = 10000;
    for batch_idx in 0..((count + BATCH_SIZE - 1) / BATCH_SIZE) {
        let batch_start = batch_idx * BATCH_SIZE;
        let batch_end = (batch_start + BATCH_SIZE).min(receiver_addresses.len());

        info!(
            "Generating batch {} ({} transactions)",
            batch_idx + 1,
            batch_end - batch_start
        );

        for i in batch_start..batch_end {
            // Pick a random sender from available addresses
            let sender_idx = if addresses.len() > 1 {
                rng.gen_range(0..addresses.len())
            } else {
                0
            };

            // Vary amount based on complexity
            let amount = if tx_type == TransactionType::Complex {
                1 + rng.gen_range(0..100 * complexity_factor)
            } else {
                1 // Minimal amount for simple transactions
            };

            // Create transaction with appropriate complexity
            let actual_complexity = match tx_type {
                TransactionType::Complex => complexity_factor,
                TransactionType::Mixed => {
                    if rng.gen_bool(0.5) {
                        complexity_factor
                    } else {
                        0
                    }
                }
                TransactionType::Transfer => 0,
            };

            transactions.push(create_mock_transaction(
                &addresses[sender_idx],
                &receiver_addresses[i],
                amount.try_into().unwrap(),
                tx_type,
                actual_complexity,
            ));

            if transactions.len() % 10000 == 0 {
                info!("Generated {} transactions", transactions.len());
            }
        }
    }

    if transactions.is_empty() {
        warn!("Failed to generate any transactions!");
    } else {
        info!("Successfully generated {} transactions", transactions.len());
    }

    transactions
}

// Benchmark modes
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum BenchmarkMode {
    /// Single-threaded submission
    Single,
    /// Multi-threaded submission
    Multi,
    /// Only measure transaction generation
    Generate,
    /// Test with simulated network conditions
    Network,
}

// Command line arguments
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Number of transactions to process
    #[arg(short = 'n', long, default_value_t = 100000)]
    transactions: usize,

    /// Number of threads for multi-threaded mode
    #[arg(short = 'j', long, default_value_t = 8)]
    threads: usize,

    /// Benchmark mode
    #[arg(short, long, value_enum, default_value_t = BenchmarkMode::Multi)]
    mode: BenchmarkMode,

    /// Target TPS goal
    #[arg(short, long, default_value_t = 100000.0)]
    target_tps: f64,

    /// Transaction type
    #[arg(long, value_enum, default_value_t = TransactionType::Transfer)]
    tx_type: TransactionType,

    /// Transaction complexity factor (0-100)
    #[arg(long, default_value_t = 0)]
    complexity: usize,

    /// Number of test accounts to create
    #[arg(long, default_value_t = 1)]
    accounts: usize,

    /// Simulate network latency in milliseconds
    #[arg(long, default_value_t = 0)]
    latency: u64,

    /// Transaction failure rate (0-100%)
    #[arg(long, default_value_t = 0.0)]
    failure_rate: f64,
}

// Simulates processing delay based on network conditions
fn simulate_network_conditions(conditions: &NetworkConditions) {
    if conditions.latency_ms > 0 {
        thread::sleep(std::time::Duration::from_millis(conditions.latency_ms));
    }
}

// Determines if a transaction should fail based on failure rate
fn should_transaction_succeed(failure_rate: f64) -> bool {
    if failure_rate <= 0.0 {
        return true;
    }
    rand::thread_rng().gen_bool(1.0 - failure_rate)
}

// Run single-threaded benchmark
fn run_single_threaded(
    transaction_count: usize,
    target_tps: f64,
    tx_type: TransactionType,
    complexity_factor: usize,
    account_count: usize,
    network_conditions: Option<NetworkConditions>,
) -> bool {
    info!(
        "Running single-threaded TPS benchmark with {} transactions",
        transaction_count
    );
    let addresses = setup_test_environment(account_count);

    // Generate transactions
    let gen_start = Instant::now();
    info!("Generating {} transactions", transaction_count);
    let transactions =
        generate_batch_transactions(&addresses, transaction_count, tx_type, complexity_factor);
    let gen_time = gen_start.elapsed();
    println!(
        "Transaction generation time: {:?} ({:.2} tx/s)",
        gen_time,
        transactions.len() as f64 / gen_time.as_secs_f64()
    );

    if transactions.is_empty() {
        error!("No transactions generated, aborting benchmark");
        return false;
    }

    // Measure submission time
    let submit_start = Instant::now();
    let mut success_count = 0;

    // Submit transactions in batches for better performance
    const BATCH_SIZE: usize = 1000;
    for (i, chunk) in transactions.chunks(BATCH_SIZE).enumerate() {
        if i % 10 == 0 {
            info!("Submitting batch {} ({} transactions)", i + 1, chunk.len());
        }
        for transaction in chunk {
            // Apply network conditions if specified
            if let Some(conditions) = &network_conditions {
                simulate_network_conditions(conditions);

                if should_transaction_succeed(conditions.failure_rate) {
                    if add_pending_transaction(transaction.clone()) {
                        success_count += 1;
                    }
                }
            } else {
                if add_pending_transaction(transaction.clone()) {
                    success_count += 1;
                }
            }
        }
    }

    let submit_time = submit_start.elapsed();
    let tps = success_count as f64 / submit_time.as_secs_f64();

    // Save state
    save_blockchain().expect("Failed to save blockchain state");

    // Print results
    println!("\n=== SINGLE-THREADED TPS BENCHMARK ===");
    println!(
        "Transaction count: {} (successful: {})",
        transactions.len(),
        success_count
    );
    println!(
        "Transaction type: {:?} (complexity: {})",
        tx_type, complexity_factor
    );
    println!("Account count: {}", account_count);
    if let Some(conditions) = &network_conditions {
        println!(
            "Network conditions: {}ms latency, {:.1}% failure rate",
            conditions.latency_ms,
            conditions.failure_rate * 100.0
        );
    }
    println!(
        "Generation time: {:?} ({:.2} tx/s)",
        gen_time,
        transactions.len() as f64 / gen_time.as_secs_f64()
    );
    println!("Submission time: {:?} ({:.2} tx/s)", submit_time, tps);
    println!("Total time: {:?}", gen_time + submit_time);
    println!("====================================");

    tps >= target_tps
}

// Run multi-threaded benchmark
fn run_multi_threaded(
    transaction_count: usize,
    thread_count: usize,
    target_tps: f64,
    tx_type: TransactionType,
    complexity_factor: usize,
    account_count: usize,
    network_conditions: Option<NetworkConditions>,
) -> bool {
    info!(
        "Running multi-threaded TPS benchmark with {} transactions on {} threads",
        transaction_count, thread_count
    );

    let addresses = setup_test_environment(account_count);

    // First generate all transactions
    let gen_start = Instant::now();
    info!("Generating {} transactions", transaction_count);
    let transactions =
        generate_batch_transactions(&addresses, transaction_count, tx_type, complexity_factor);
    let gen_time = gen_start.elapsed();

    println!(
        "Transaction generation time: {:?} ({:.2} tx/s)",
        gen_time,
        transactions.len() as f64 / gen_time.as_secs_f64()
    );

    if transactions.is_empty() {
        error!("No transactions generated, aborting benchmark");
        return false;
    }

    // Prepare for multi-threaded submission
    let transactions_arc = Arc::new(transactions);
    let success_counter = Arc::new(Mutex::new(0));
    let network_conditions_arc = network_conditions.map(Arc::new);

    // Calculate chunk size per thread
    let chunk_size = (transactions_arc.len() + thread_count - 1) / thread_count;

    // Start measuring submission time
    let submit_start = Instant::now();
    let mut handles = Vec::with_capacity(thread_count);

    // Create threads
    for i in 0..thread_count {
        let transactions = Arc::clone(&transactions_arc);
        let counter = Arc::clone(&success_counter);
        let conditions = network_conditions_arc.clone();
        let start_idx = i * chunk_size;
        let end_idx = (start_idx + chunk_size).min(transactions.len());

        let handle = thread::spawn(move || {
            let mut local_success = 0;

            for j in start_idx..end_idx {
                // Apply network conditions if specified
                if let Some(conditions) = &conditions {
                    simulate_network_conditions(conditions.as_ref());

                    if should_transaction_succeed(conditions.failure_rate) {
                        if add_pending_transaction(transactions[j].clone()) {
                            local_success += 1;
                        }
                    }
                } else {
                    if add_pending_transaction(transactions[j].clone()) {
                        local_success += 1;
                    }
                }
            }

            // Update global counter
            let mut global_counter = counter.lock().unwrap();
            *global_counter += local_success;
        });

        handles.push(handle);
    }

    // Wait for all threads
    for handle in handles {
        let _ = handle.join();
    }

    let submit_time = submit_start.elapsed();
    let success_count = *success_counter.lock().unwrap();
    let tps = success_count as f64 / submit_time.as_secs_f64();

    // Save state
    save_blockchain().expect("Failed to save blockchain state");

    // Print results
    println!("\n=== MULTI-THREADED TPS BENCHMARK ===");
    println!(
        "Transaction count: {} (successful: {})",
        transactions_arc.len(),
        success_count
    );
    println!("Thread count: {}", thread_count);
    println!(
        "Transaction type: {:?} (complexity: {})",
        tx_type, complexity_factor
    );
    println!("Account count: {}", account_count);
    if let Some(conditions) = &network_conditions_arc {
        println!(
            "Network conditions: {}ms latency, {:.1}% failure rate",
            conditions.latency_ms,
            conditions.failure_rate * 100.0
        );
    }
    println!(
        "Generation time: {:?} ({:.2} tx/s)",
        gen_time,
        transactions_arc.len() as f64 / gen_time.as_secs_f64()
    );
    println!("Submission time: {:?} ({:.2} tx/s)", submit_time, tps);
    println!("Total time: {:?}", gen_time + submit_time);
    println!("====================================");

    tps >= target_tps
}

// Only measure transaction generation (no submission)
fn run_generation_only(
    transaction_count: usize,
    tx_type: TransactionType,
    complexity_factor: usize,
    account_count: usize,
) {
    info!(
        "Measuring transaction generation performance for {} transactions",
        transaction_count
    );

    let addresses = setup_test_environment(account_count);

    let gen_start = Instant::now();
    info!("Generating {} transactions", transaction_count);
    let transactions =
        generate_batch_transactions(&addresses, transaction_count, tx_type, complexity_factor);
    let gen_time = gen_start.elapsed();

    println!("\n=== TRANSACTION GENERATION BENCHMARK ===");
    println!(
        "Transaction count: {} (generated: {})",
        transaction_count,
        transactions.len()
    );
    println!(
        "Transaction type: {:?} (complexity: {})",
        tx_type, complexity_factor
    );
    println!("Account count: {}", account_count);
    println!("Generation time: {:?}", gen_time);
    println!(
        "Rate: {:.2} transactions per second",
        transactions.len() as f64 / gen_time.as_secs_f64()
    );
    println!("=========================================");
}

// Run benchmark with simulated network conditions
fn run_network_simulation(
    transaction_count: usize,
    thread_count: usize,
    target_tps: f64,
    tx_type: TransactionType,
    complexity_factor: usize,
    account_count: usize,
    network_conditions: NetworkConditions,
) -> bool {
    info!(
        "Running network simulation benchmark with {}ms latency and {:.1}% failure rate",
        network_conditions.latency_ms,
        network_conditions.failure_rate * 100.0
    );

    // Use multi-threaded mode with network conditions
    run_multi_threaded(
        transaction_count,
        thread_count,
        target_tps,
        tx_type,
        complexity_factor,
        account_count,
        Some(network_conditions),
    )
}

#[tokio::main]
async fn main() {
    // Parse command line arguments
    let args = Args::parse();

    // Initialize logging
    init_logger();
    info!("Starting TPS benchmark in {:?} mode", args.mode);

    // Validate inputs
    let complexity_factor = args.complexity.min(100); // Cap complexity at 100
    let failure_rate = (args.failure_rate / 100.0).min(1.0); // Convert percentage to 0.0-1.0

    // Create network conditions if specified
    let network_conditions = if args.latency > 0 || failure_rate > 0.0 {
        Some(NetworkConditions {
            latency_ms: args.latency,
            failure_rate,
        })
    } else {
        None
    };

    // Run the appropriate benchmark
    let success = match args.mode {
        BenchmarkMode::Single => run_single_threaded(
            args.transactions,
            args.target_tps,
            args.tx_type,
            complexity_factor,
            args.accounts,
            network_conditions,
        ),
        BenchmarkMode::Multi => run_multi_threaded(
            args.transactions,
            args.threads,
            args.target_tps,
            args.tx_type,
            complexity_factor,
            args.accounts,
            network_conditions,
        ),
        BenchmarkMode::Generate => {
            run_generation_only(
                args.transactions,
                args.tx_type,
                complexity_factor,
                args.accounts,
            );
            true
        }
        BenchmarkMode::Network => {
            // Ensure we have network conditions
            let conditions = network_conditions.unwrap_or(NetworkConditions {
                latency_ms: 50,     // Default latency if none provided
                failure_rate: 0.05, // Default 5% failure rate
            });

            run_network_simulation(
                args.transactions,
                args.threads,
                args.target_tps,
                args.tx_type,
                complexity_factor,
                args.accounts,
                conditions,
            )
        }
    };

    if success {
        println!(
            "\n✅ TARGET ACHIEVED: The benchmark met or exceeded the target of {:.0} TPS!",
            args.target_tps
        );
    } else {
        println!(
            "\n⚠️ TARGET NOT REACHED: The benchmark did not reach the target of {:.0} TPS.",
            args.target_tps
        );
    }
}
