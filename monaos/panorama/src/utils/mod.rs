use mona_types::Transaction;
use tokio::sync::mpsc;
use log::{info, error};
use rand::{Rng, thread_rng};

use crate::simulation::add_pending_transaction;
// Add mona-crypto imports for enhanced security
use mona_crypto::{hash_data_blake3, HashAlgorithm, hash_data_with_algorithm, secure_clear};

// Make the gas module public
pub mod gas {
    pub fn calculate_gas_fee(priority_boost: Option<u64>) -> u64 {
        mona_types::gas::calculate_gas_fee(priority_boost)
    }

    pub fn format_gas_fee_display(fee: u64) -> String {
        mona_types::gas::format_gas_fee_display(fee)
    }

    pub fn calculate_total_transaction_cost(amount: u64, gas_fee: u64) -> u64 {
        mona_types::gas::calculate_total_transaction_cost(amount, gas_fee)
    }
}

// Re-export gas items for backward compatibility
pub use mona_types::gas::*;

/// Generate a cryptographically secure transaction ID using Blake3
pub fn generate_transaction_id() -> String {
    // Create unique content to hash (timestamp + random data)
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    let mut rng = thread_rng();
    let random_value: u64 = rng.r#gen();
    
    // Combine timestamp and random value
    let mut content = Vec::with_capacity(16);
    content.extend_from_slice(&timestamp.to_le_bytes());
    content.extend_from_slice(&random_value.to_le_bytes());
    
    // Hash the content using Blake3 (faster and more secure)
    let hash_result = hash_data_blake3(&content);
    
    // Format as hex string with 0x prefix
    format!("0x{}", hex::encode(&hash_result[0..32]))
}

/// Process a blockchain transfer with improved security
pub fn process_blockchain_transfer(
    from_address: &str,
    to_address: &str,
    amount: u64,
    password: &str,
    priority_boost: Option<u64>,
    notification_channel: Option<&mpsc::Sender<String>>
) -> Result<Transaction, String> {
    // Calculate the gas fee dynamically
    let gas_fee = calculate_gas_fee(priority_boost);
    
    // Create a copy of the password for secure handling
    let password_copy = password.to_string();
    
    // Perform the transfer
    let result = match crate::transfer_tokens::transfer_tokens(from_address, to_address, amount, &password_copy, gas_fee) {
        Ok(transaction) => {
            // Add to pending transactions queue
            if add_pending_transaction(transaction.clone()) {
                info!("Transaction added to pending queue: {} -> {}, amount: {}, gas fee: {}", 
                      transaction.sender, transaction.receiver, amount, gas_fee);
                
                // Send notification if channel provided
                if let Some(tx) = notification_channel {
                    let tx_json = serde_json::json!({
                        "event": "transaction_created",
                        "transaction": {
                            "id": transaction.transaction_id,
                            "sender": transaction.sender.to_hex_literal(),
                            "receiver": transaction.receiver.to_hex_literal(),
                            "amount": amount,
                            "gas_fee": gas_fee,
                            "timestamp": transaction.timestamp,
                            "signed": !transaction.signature.is_empty()
                        },
                        "status": "pending"
                    }).to_string();
                    
                    let _ = tx.try_send(tx_json);
                }
                
                Ok(transaction)
            } else {
                error!("Failed to add transaction to pending queue");
                Err("Failed to add transaction to pending queue".to_string())
            }
        },
        Err(e) => {
            error!("Transaction failed: {}", e);
            
            // Send notification if channel provided
            if let Some(tx) = notification_channel {
                let error_json = serde_json::json!({
                    "event": "transaction_error",
                    "error": format!("{}", e),
                    "details": {
                        "sender": from_address,
                        "receiver": to_address,
                        "amount": amount,
                        "gas_fee": gas_fee,
                    }
                }).to_string();
                
                let _ = tx.try_send(error_json);
            }
            
            Err(format!("Transaction failed: {}", e))
        }
    };
    
    // Securely clear the password copy
    let mut password_bytes = password_copy.into_bytes();
    secure_clear(&mut password_bytes);
    
    result
}

/// Generate a cryptographically secure hash for any data using Blake3
pub fn secure_hash(data: &[u8]) -> Vec<u8> {
    hash_data_blake3(data)
}

/// Generate a deterministic hash for data with algorithm selection
pub fn hash_with_algorithm(data: &[u8], algorithm: HashAlgorithm) -> Vec<u8> {
    hash_data_with_algorithm(data, algorithm)
}

/// Format KA amount to display as KARI with proper formatting
pub fn format_kari_amount(ka_amount: u64) -> String {
    const KA_PER_KARI: u64 = 1_000_000_000;
    
    // Calculate whole and fractional parts
    let whole_kari = ka_amount / KA_PER_KARI;
    let fractional_ka = ka_amount % KA_PER_KARI;
    
    // Format with thousands separators and 9 decimal places
    let whole_formatted = format!("{}", whole_kari)
        .chars()
        .rev()
        .collect::<Vec<_>>()
        .chunks(3)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(",")
        .chars()
        .rev()
        .collect::<String>();
    
    format!("{}.{:09}", whole_formatted, fractional_ka)
}
