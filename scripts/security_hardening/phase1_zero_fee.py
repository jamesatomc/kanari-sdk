from __future__ import annotations

from .common import read, write


def apply() -> None:
    path = "crates/kanari-types/src/gas_v1.rs"
    text = read(path)
    old = '''    pub fn validate_price(&self, gas_price: u64) -> Result<(), GasError> {
        if gas_price < self.min_gas_price {
            return Err(GasError::PriceTooLow {
                provided: gas_price,
                minimum: self.min_gas_price,
            });
        }
        Ok(())
    }'''
    new = '''    pub fn validate_price(&self, gas_price: u64) -> Result<(), GasError> {
        if gas_price != 0 {
            return Err(GasError::PriceMismatch {
                provided: gas_price,
                required: 0,
            });
        }
        Ok(())
    }'''
    if old not in text:
        raise RuntimeError("gas price validation block not found")
    text = text.replace(old, new, 1)
    text = text.replace(
        '''            base_price: 1,                // 1 Mist per gas unit (extremely low)
            max_gas_per_tx: 100_000,      // 100K gas per transaction
            max_gas_per_block: 1_000_000, // 1M gas per block
            min_gas_price: 1,             // 1 Mist minimum
            storage_price_per_byte: 1,    // 1 Mist per byte (extremely low)''',
        '''            base_price: 0,                // execution is metered but protocol fees are zero
            max_gas_per_tx: 100_000,      // 100K gas per transaction
            max_gas_per_block: 1_000_000, // 1M gas per block
            min_gas_price: 0,             // every transaction must declare zero price
            storage_price_per_byte: 0,    // storage is bounded by gas/byte limits, not token fees''',
        1,
    )
    text = text.replace(
        '''    PriceTooLow { provided: u64, minimum: u64 },
    Overflow,''',
        '''    PriceTooLow { provided: u64, minimum: u64 },
    PriceMismatch { provided: u64, required: u64 },
    Overflow,''',
        1,
    )
    text = text.replace(
        '''            GasError::PriceTooLow { provided, minimum } => {
                write!(
                    f,
                    "Gas price too low: provided {} but minimum is {}",
                    provided, minimum
                )
            }
            GasError::Overflow''',
        '''            GasError::PriceTooLow { provided, minimum } => {
                write!(
                    f,
                    "Gas price too low: provided {} but minimum is {}",
                    provided, minimum
                )
            }
            GasError::PriceMismatch { provided, required } => {
                write!(f, "Invalid gas price: provided {} but protocol requires {}", provided, required)
            }
            GasError::Overflow''',
        1,
    )
    text = text.replace(
        '''    fn gas_config_rejects_price_below_minimum() {
        let config = GasConfig::default();

        assert!(matches!(
            config.validate_price(0),
            Err(GasError::PriceTooLow { .. })
        ));
        assert!(config.validate_price(config.min_gas_price).is_ok());
    }''',
        '''    fn gas_config_requires_zero_protocol_price() {
        let config = GasConfig::default();

        assert!(config.validate_price(0).is_ok());
        assert!(matches!(
            config.validate_price(1),
            Err(GasError::PriceMismatch { .. })
        ));
    }''',
        1,
    )
    write(path, text)

    path = "crates/kanari-auth/src/auth_manager.rs"
    text = read(path)
    text = text.replace(
        "*tx_gas_price = gas_price.unwrap_or(1_000);",
        "*tx_gas_price = gas_price.unwrap_or(0);",
        1,
    )
    write(path, text)
