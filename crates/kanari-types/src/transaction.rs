// Blockchain data structures and operations
use anyhow::Result;
use kanari_crypto::keys::CurveType;
use kanari_crypto::verify_signature;
use kanari_crypto::{hash_data_blake3, signatures::sign_message};
use move_core_types::account_address::AccountAddress;
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::digest::{Digest, TransactionDigest};

/// Signed transaction wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedTransaction {
    pub transaction: Transaction,
    pub signature: Vec<u8>,
}

impl SignedTransaction {
    pub fn new(transaction: Transaction) -> Self {
        Self {
            transaction,
            signature: vec![],
        }
    }

    pub fn sign(&mut self, private_key: &str, curve_type: CurveType) -> Result<()> {
        let tx_hash = self.transaction.hash();
        let signature = sign_message(private_key, &tx_hash.0.0, curve_type)
            .map_err(|e| anyhow::anyhow!("Failed to sign transaction: {}", e))?;
        self.signature = signature;
        Ok(())
    }

    pub fn verify_signature(&self) -> Result<bool> {
        if self.signature.is_empty() {
            anyhow::bail!("Transaction not signed");
        }

        let signature = &self.signature;
        let tx_hash = self.transaction.hash();
        let sender = self.transaction.sender();

        verify_signature(sender, &tx_hash.0.0, signature)
            .map_err(|e| anyhow::anyhow!("Signature verification failed: {}", e))
    }

    pub fn hash(&self) -> TransactionDigest {
        let serialized = bcs::to_bytes(self).unwrap_or_default();
        let hash_bytes = kanari_crypto::hash_data_blake3(&serialized);

        // แปลง Vec<u8> 32 bytes ให้เป็น TransactionDigest อย่างปลอดภัย
        TransactionDigest(Digest::from_bytes(&hash_bytes).expect("Hash must be 32 bytes"))
    }
}

/// Transaction types in Kanari blockchain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Transaction {
    /// Publish a Move module
    PublishModule {
        sender: String,
        module_bytes: Vec<u8>,
        module_name: String,
        gas_limit: u64,
        gas_price: u64,
        sequence_number: u64,
    },
    /// Execute a Move function
    ExecuteFunction {
        sender: String,
        module: String,
        function: String,
        type_args: Vec<String>,
        args: Vec<Vec<u8>>,
        gas_limit: u64,
        gas_price: u64,
        sequence_number: u64,
    },
    /// Transfer coins
    Transfer {
        from: String,
        to: String,
        amount: u64,
        gas_limit: u64,
        gas_price: u64,
        sequence_number: u64,
    },
    /// Burn coins (remove from total supply)
    Burn {
        from: String,
        amount: u64,
        gas_limit: u64,
        gas_price: u64,
        sequence_number: u64,
    },
}

impl Transaction {
    pub fn hash(&self) -> TransactionDigest {
        let serialized = match bcs::to_bytes(self) {
            Ok(b) => b,
            Err(e) => {
                error!("Failed to serialize Transaction for hashing: {}", e);
                vec![0u8; 32]
            }
        };
        let hash_bytes = hash_data_blake3(&serialized);

        // คืนค่าเป็น Strongly Typed
        TransactionDigest(Digest::from_bytes(&hash_bytes).expect("Hash must be 32 bytes"))
    }

    pub fn sender(&self) -> &str {
        match self {
            Transaction::PublishModule { sender, .. } => sender,
            Transaction::ExecuteFunction { sender, .. } => sender,
            Transaction::Transfer { from, .. } => from,
            Transaction::Burn { from, .. } => from,
        }
    }

    pub fn sender_address(&self) -> &str {
        self.sender()
    }

    pub fn sequence_number(&self) -> u64 {
        match self {
            Transaction::PublishModule {
                sequence_number, ..
            } => *sequence_number,
            Transaction::ExecuteFunction {
                sequence_number, ..
            } => *sequence_number,
            Transaction::Transfer {
                sequence_number, ..
            } => *sequence_number,
            Transaction::Burn {
                sequence_number, ..
            } => *sequence_number,
        }
    }

    pub fn gas_limit(&self) -> u64 {
        match self {
            Transaction::PublishModule { gas_limit, .. } => *gas_limit,
            Transaction::ExecuteFunction { gas_limit, .. } => *gas_limit,
            Transaction::Transfer { gas_limit, .. } => *gas_limit,
            Transaction::Burn { gas_limit, .. } => *gas_limit,
        }
    }

    pub fn gas_price(&self) -> u64 {
        match self {
            Transaction::PublishModule { gas_price, .. } => *gas_price,
            Transaction::ExecuteFunction { gas_price, .. } => *gas_price,
            Transaction::Transfer { gas_price, .. } => *gas_price,
            Transaction::Burn { gas_price, .. } => *gas_price,
        }
    }

    /// Get conflict keys for this transaction.
    /// Transactions with overlapping conflict keys must be executed sequentially.
    pub fn get_conflict_keys(&self) -> Vec<String> {
        // 1. บังคับ Normalize Sender (แปลงเป็น Address มาตรฐาน ตัวพิมพ์เล็ก มี 0x เสมอ)
        let sender_norm = if let Ok(addr) = AccountAddress::from_hex_literal(self.sender()) {
            addr.to_hex_literal()
        } else {
            let s = self.sender();
            if !s.starts_with("0x") {
                format!("0x{}", s)
            } else {
                s.to_string()
            }
        };

        let mut keys = vec![sender_norm];

        match self {
            Transaction::Transfer { to, .. } => {
                // 2. บังคับ Normalize ปลายทาง
                let to_norm = if let Ok(addr) = AccountAddress::from_hex_literal(to) {
                    addr.to_hex_literal()
                } else if !to.starts_with("0x") {
                    format!("0x{}", to)
                } else {
                    to.to_string()
                };
                keys.push(to_norm);
            }
            Transaction::ExecuteFunction { args, .. } => {
                for arg in args {
                    if arg.len() == 32
                        && let Ok(addr) = AccountAddress::from_bytes(arg)
                    {
                        keys.push(addr.to_hex_literal());
                        continue; // ข้ามไปตัวถัดไปถ้าตรงเงื่อนไขแล้ว
                    }

                    // 2. ดึง String ออกมาจาก Argument (รองรับทั้งแบบ BCS String และ Raw UTF-8)
                    let parsed_string = bcs::from_bytes::<String>(arg)
                        .or_else(|_| std::str::from_utf8(arg).map(|s| s.to_string()));

                    if let Ok(s) = parsed_string {
                        let s_trim = s.trim();

                        // บังคับเติม 0x ถ้ายังไม่มี
                        let hex_str = if !s_trim.starts_with("0x") {
                            format!("0x{}", s_trim)
                        } else {
                            s_trim.to_string()
                        };

                        // ใช้ from_hex_literal เพื่อจัดการความยาวและแปลงเป็น Lowercase
                        if let Ok(addr) = AccountAddress::from_hex_literal(&hex_str) {
                            keys.push(addr.to_hex_literal());
                        }
                    }
                }
            }
            Transaction::PublishModule { module_name, .. } => {
                keys.push(module_name.clone());
            }
            Transaction::Burn { .. } => {
                // ไม่ต้องทำอะไรเพิ่มสำหรับ Burn
            }
        }
        keys
    }

    /// Create a transfer transaction with default gas settings
    pub fn new_transfer(from: String, to: String, amount: u64, sequence_number: u64) -> Self {
        Self::Transfer {
            from,
            to,
            amount,
            gas_limit: 100_000, // Default gas limit
            gas_price: 1000,    // Default gas price (1000 Mist)
            sequence_number,
        }
    }

    /// Create a burn transaction with default gas settings
    pub fn new_burn(from: String, amount: u64, sequence_number: u64) -> Self {
        Self::Burn {
            from,
            amount,
            gas_limit: 100_000,
            gas_price: 1000,
            sequence_number,
        }
    }
}
