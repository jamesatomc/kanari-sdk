# Unit Tests & Integration Tests for kanari-move-runtime

## Overview

This document describes the comprehensive test suite implemented for `kanari-move-runtime` to achieve >90% code coverage and ensure system reliability through fuzz testing.

## Test Files Created

### 1. `tests/unit_tests.rs` - Comprehensive Unit Tests

Covers all core modules with detailed unit tests:

#### ChangeSet Tests
- `test_changeset_new` - Verify empty ChangeSet initialization
- `test_account_change_new` - Verify AccountChange initialization
- `test_account_change_debit_credit` - Test balance debit/credit operations
- `test_account_change_sequence_increment` - Test sequence number increments
- `test_account_change_add_module` - Test module addition (with duplicate handling)
- `test_changeset_add_token_balance_set` - Test token balance additions
- `test_changeset_add_treasury` - Test treasury cap additions

#### Account Tests
- `test_account_new` - Verify Account initialization
- `test_account_add_module` - Test module additions
- `test_account_set_get_token_balance` - Test token balance get/set
- `test_account_to_hex_string` - Test hex string conversion
- `test_account_increment_sequence` - Test sequence increment

#### Scheduler Tests
- `test_scheduler_empty_input` - Empty transaction list
- `test_scheduler_single_transaction` - Single transaction scheduling
- `test_scheduler_no_conflicts` - Multiple independent transactions
- `test_scheduler_with_object_conflicts` - Transactions with object conflicts
- `test_scheduler_sequential_same_account` - Same-account sequential ordering

#### PersistentStore Tests
- `test_persistent_store_in_memory` - In-memory store operations
- `test_persistent_store_serialization` - BCS serialization/deserialization
- `test_persistent_store_delete` - Key deletion
- `test_persistent_store_nonexistent_key` - Non-existent key handling
- `test_persistent_store_flush` - Flush operations

#### Gas Meter Tests
- `test_gas_meter_creation` - Gas meter initialization
- `test_gas_meter_charge_step` - Step charging
- `test_gas_meter_charge_multiple_steps` - Multiple charges
- `test_gas_meter_out_of_gas` - Out of gas error handling
- `test_gas_meter_exact_limit` - Exact limit boundary
- `test_gas_meter_remaining_gas` - Remaining gas calculation
- `test_gas_meter_saturating_add` - Saturating arithmetic

#### CreatedObject Tests
- `test_created_object_basic` - Basic object creation
- `test_created_object_with_uid` - Object with UID record

### 2. `tests/fuzz_tests.rs` - Fuzz Testing Suite

Random input testing to find edge cases and potential crashes:

#### Helper Functions
- `random_account_address` - Generate random AccountAddress
- `random_string` - Generate random strings
- `random_bytes` - Generate random byte vectors

#### ChangeSet Fuzz Tests
- `fuzz_changeset_with_random_data` - Random data in changesets
- `fuzz_account_change_with_extreme_values` - Extreme debit/credit values
- `fuzz_changeset_many_operations` - Many operations stress test

#### Account Fuzz Tests
- `fuzz_account_with_random_balances` - Random token balances
- `fuzz_account_module_additions` - Random module additions

#### Scheduler Fuzz Tests
- `fuzz_scheduler_with_random_transactions` - Random transaction scheduling
- `fuzz_scheduler_same_account_sequential` - Sequential ordering verification

#### PersistentStore Fuzz Tests
- `fuzz_persistent_store_random_keys_values` - Random keys/values
- `fuzz_persistent_store_delete_random` - Random deletions
- `fuzz_persistent_store_large_values` - Large value storage

#### Gas Meter Fuzz Tests
- `fuzz_gas_meter_random_charges` - Random charge patterns
- `fuzz_gas_meter_edge_cases` - Edge case limits (0, 1, u64::MAX)

#### StateManager Fuzz Tests
- `fuzz_statemanager_apply_changesets` - Random changeset applications
- `fuzz_statemanager_concurrent_reads` - Concurrent read operations

#### Serialization Fuzz Tests
- `fuzz_created_object_random_data` - Random object data
- `fuzz_changeset_serialization_roundtrip` - BCS roundtrip verification

## Running Tests

### Run All Tests
```bash
cargo test -p kanari-move-runtime
```

### Run Unit Tests Only
```bash
cargo test -p kanari-move-runtime --test unit_tests
```

### Run Fuzz Tests Only
```bash
cargo test -p kanari-move-runtime --test fuzz_tests
```

### Run with Coverage Report
```bash
cargo install cargo-tarpaulin
cargo tarpaulin -p kanari-move-runtime --out Html
```

### Run Specific Test
```bash
cargo test -p kanari-move-runtime test_scheduler_with_object_conflicts
```

## Code Coverage Goals

| Module | Target Coverage | Status |
|--------|----------------|--------|
| changeset.rs | 95% | ✅ Covered |
| state.rs | 90% | ✅ Covered |
| scheduler.rs | 95% | ✅ Covered |
| kanari_gas_meter.rs | 95% | ✅ Covered |
| storage/persistent_store.rs | 90% | ✅ Covered |
| move_runtime/mod.rs | 85% | Partial (requires Move VM integration) |

## Test Categories

### 1. Unit Tests
- Test individual functions and methods
- Verify expected behavior with known inputs
- Check edge cases and boundary conditions

### 2. Integration Tests
- Test interactions between components
- Verify ChangeSet application to StateManager
- Test end-to-end transaction scheduling

### 3. Fuzz Tests
- Generate random inputs automatically
- Find unexpected panics or crashes
- Test with extreme values (0, u64::MAX, etc.)
- Verify serialization roundtrips

### 4. Property-Based Tests
- Verify invariants hold for all inputs
- Example: "Same-account transactions are always sequential"
- Example: "Serialization roundtrip preserves data"

## Key Test Scenarios for DeFi

### Token Operations
- Multiple mint operations consolidation
- Balance transfers between accounts
- Treasury cap creation and management

### Object Operations
- NFT creation and ownership transfer
- Object deletion and cleanup
- Version tracking

### Concurrency
- Parallel transaction execution waves
- Conflict detection and resolution
- Race condition prevention

### Gas Management
- Step counting accuracy
- Out-of-gas handling
- Saturating arithmetic safety

## Known Limitations

1. **Move VM Integration Tests**: Full integration tests requiring Move bytecode compilation need the complete Move toolchain installed.

2. **Persistent RocksDB Tests**: Some tests use in-memory stores; full RocksDB persistence tests require disk space.

3. **Network Tests**: P2P and RPC tests are in separate crates (kanari-node).

## Recommendations for Production

1. **Run CI/CD**: Execute all tests on every PR
2. **Coverage Threshold**: Fail builds if coverage drops below 90%
3. **Fuzz Testing**: Run fuzz tests continuously in background
4. **Property Testing**: Add more property-based tests for critical invariants
5. **Integration Tests**: Add end-to-end tests with real Move modules

## Dependencies Added

For testing, the following dev-dependencies were added to `Cargo.toml`:
- `rand = "0.10.1"` - Random number generation for fuzz testing
- `hex = "0.4.3"` - Hex encoding/decoding
- `bcs = "0.2.1"` - Binary Canonical Serialization for roundtrip tests
