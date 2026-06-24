from __future__ import annotations

import re

from .common import insert_after_once, read, regex_once, write


def apply() -> None:
    path = "crates/kanari-types/src/transaction.rs"
    regex_once(
        path,
        r"    pub fn native_call\(&self\) -> Option<NativeCall> \{.*?\n    \}\n\n    pub fn is_native_balance_call\(&self\) -> bool \{\n        self\.native_call\(\)\.is_some\(\)\n    \}",
        '''    /// Legacy account-ledger balance calls are intentionally disabled.
    ///
    /// Native KANARI balances are canonical `Coin<KANARI>` objects. Intercepting
    /// `transfer_amount` or `burn_amount` in Rust would mutate account balances
    /// without mutating the backing coin object and permits payment reversal.
    pub fn native_call(&self) -> Option<NativeCall> {
        None
    }

    pub fn is_legacy_native_balance_call(&self) -> bool {
        matches!(
            self,
            Transaction::ExecuteFunction { module, function, args, .. }
                if module == Self::KANARI_MODULE
                    && ((function == Self::TRANSFER_AMOUNT_FUNCTION && args.len() == 2)
                        || (function == Self::BURN_AMOUNT_FUNCTION && args.len() == 1))
        )
    }

    pub fn is_native_balance_call(&self) -> bool {
        false
    }''',
        re.S,
    )

    marker = '''    pub fn new_transfer_with_gas(
        from: String,
        to: String,
        amount: u64,
        sequence_number: u64,
        gas_limit: u64,
        gas_price: u64,
    ) -> Self {
        Self::ExecuteFunction {
            sender: from,
            module: Self::KANARI_MODULE.to_string(),
            function: Self::TRANSFER_AMOUNT_FUNCTION.to_string(),
            type_args: vec![],
            args: vec![
                bcs::to_bytes(&amount).unwrap_or_default(),
                bcs::to_bytes(&to).unwrap_or_default(),
            ],
            gas_limit,
            gas_price,
            sequence_number,
        }
    }
'''
    insert_after_once(
        path,
        marker,
        '''
    /// Build a canonical Move-object transfer. The object id identifies the
    /// sender-owned `Coin<KANARI>` that will be split by `kanari::transfer_amount`.
    pub fn new_object_transfer(
        from: String,
        coin_object_id: AccountAddress,
        to: AccountAddress,
        amount: u64,
        sequence_number: u64,
    ) -> Self {
        let gas = crate::gas::GasConfig::default();
        Self::new_object_transfer_with_gas(
            from,
            coin_object_id,
            to,
            amount,
            sequence_number,
            gas.default_transaction_gas_limit(),
            gas.default_transaction_gas_price(),
        )
    }

    pub fn new_object_transfer_with_gas(
        from: String,
        coin_object_id: AccountAddress,
        to: AccountAddress,
        amount: u64,
        sequence_number: u64,
        gas_limit: u64,
        gas_price: u64,
    ) -> Self {
        Self::ExecuteFunction {
            sender: from,
            module: Self::KANARI_MODULE.to_string(),
            function: Self::TRANSFER_AMOUNT_FUNCTION.to_string(),
            type_args: vec![],
            args: vec![
                bcs::to_bytes(&coin_object_id).unwrap_or_default(),
                bcs::to_bytes(&amount).unwrap_or_default(),
                bcs::to_bytes(&to).unwrap_or_default(),
            ],
            gas_limit,
            gas_price,
            sequence_number,
        }
    }
''',
    )

    text = read(path)
    text = text.replace(
        '''        assert_eq!(
            tx.native_call(),
            Some(NativeCall::TransferAmount {
                recipient: "0x2".to_string(),
                amount: 42,
            })
        );
        assert_eq!(tx.tx_type_label(), "transfer");''',
        '''        assert!(tx.native_call().is_none());
        assert!(tx.is_legacy_native_balance_call());
        assert_eq!(tx.tx_type_label(), "call");''',
    )
    text = text.replace(
        '''        assert_eq!(tx.native_call(), Some(NativeCall::BurnAmount { amount: 9 }));
        assert_eq!(tx.tx_type_label(), "burn");''',
        '''        assert!(tx.native_call().is_none());
        assert!(tx.is_legacy_native_balance_call());
        assert_eq!(tx.tx_type_label(), "call");''',
    )
    write(path, text)

    write(
        "crates/kanari-types/src/gas.rs",
        '''//! Consensus gas schedule.
//!
//! The active schedule must not depend on Cargo features because validators built
//! with different features would disagree on transaction validity and state roots.
pub use crate::gas_v1::*;
''',
    )
    lib_path = "crates/kanari-types/src/lib.rs"
    lib = read(lib_path)
    lib, count = re.subn(
        r"pub mod gas;\n#\[cfg\(not\(feature = \"zero-gas\"\)\)\]\nmod gas_v1;\n#\[cfg\(feature = \"zero-gas\"\)\]\nmod gas_v2;",
        'pub mod gas;\nmod gas_v1;\n#[cfg(feature = "zero-gas")]\nmod gas_v2;',
        lib,
        count=1,
    )
    if count != 1:
        raise RuntimeError("kanari-types gas module selector not found")
    write(lib_path, lib)

    gas_path = "crates/kanari-types/src/gas_v1.rs"
    insert_after_once(
        gas_path,
        "use serde::{Deserialize, Serialize};\n",
        '''
/// Version of the deterministic, consensus-critical gas schedule.
pub const GAS_SCHEDULE_VERSION: u64 = 1;

''',
    )
    insert_after_once(
        gas_path,
        '''    pub fn validate_price(&self, gas_price: u64) -> Result<(), GasError> {
        if gas_price < self.min_gas_price {
            return Err(GasError::PriceTooLow {
                provided: gas_price,
                minimum: self.min_gas_price,
            });
        }
        Ok(())
    }
''',
        '''
    /// Stable digest committed into every checkpoint.
    pub fn consensus_hash(&self) -> [u8; 32] {
        let bytes = bcs::to_bytes(&(
            b"kanari:gas-schedule:v1".as_slice(),
            GAS_SCHEDULE_VERSION,
            self.base_price,
            self.max_gas_per_tx,
            self.max_gas_per_block,
            self.min_gas_price,
            self.storage_price_per_byte,
            self.storage_rebate_rate,
        ))
        .expect("gas schedule serialization is infallible");
        let digest = kanari_crypto::hash_data_blake3(&bytes);
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest[..32]);
        out
    }
''',
    )
