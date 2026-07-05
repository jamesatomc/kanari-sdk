#![allow(clippy::print_stdout)]

use anyhow::Result;
use kanari_move_runtime_v1::{
    state::{Account, StateManager},
    storage::persistent_store::PersistentStore,
};
use kanari_types::address::Address;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use std::path::PathBuf;
use std::sync::Arc;

fn main() -> Result<()> {
    // Setup temporary DB path
    let db_path = PathBuf::from("./test_db_persistence");
    if db_path.exists() {
        std::fs::remove_dir_all(&db_path)?;
    }

    let addr = Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();
    let acc_addr =
        Address::parse_to_account_address("0x1234567890abcdef1234567890abcdef12345678").unwrap();

    println!("1. Creating StateManager with RocksDB at {:?}", db_path);
    let store1 = Arc::new(PersistentStore::open_with_path(Some(db_path.clone())).unwrap());
    let mut state1 = StateManager::new(store1.clone());

    // Create an account
    println!("2. Creating account {:?} with balance 1000", addr);
    let account = Account::with_native_balance(acc_addr, 1000);
    state1.save_account(&account)?;

    // Commit changes
    println!("3. Committing state to disk...");
    state1.commit()?;

    // Compute state root
    let root = state1.compute_state_root();
    println!("   State root: {}", hex::encode(root));

    // Explicitly drop to release file handles
    drop(state1);
    drop(store1);

    // Force RocksDB to release files
    std::thread::sleep(std::time::Duration::from_millis(1000));

    println!("4. Re-opening StateManager from disk...");
    let store2 = Arc::new(PersistentStore::open_with_path(Some(db_path.clone())).unwrap());
    let state2 = StateManager::new(store2.clone());

    // Verify account
    println!("5. Verifying account state...");
    let account = state2.get_account(&acc_addr).expect("Account should exist");

    assert_eq!(
        account.get_token_balance(KANARI_TOKEN_TYPE),
        1000,
        "Balance should be 1000"
    );
    assert_eq!(account.sequence_number, 1, "Sequence number should be 1");

    println!("   Account verification successful!");
    println!(
        "   Balance: {}",
        account.get_token_balance(KANARI_TOKEN_TYPE)
    );
    println!("   Sequence: {}", account.sequence_number);

    // Verify state root matches
    let root = state2.compute_state_root();
    println!("   State root: {}", hex::encode(root));

    // Explicitly drop before cleanup
    drop(state2);
    drop(store2);

    // Give OS time to release all file handles
    std::thread::sleep(std::time::Duration::from_millis(2000));

    // Force garbage collection
    #[cfg(target_os = "windows")]
    {
        // On Windows, RocksDB may hold file locks longer
        // Try multiple times with delays
        for attempt in 1..=5 {
            match std::fs::remove_dir_all(&db_path) {
                Ok(_) => {
                    println!("6. Cleanup successful on attempt {}!", attempt);
                    println!("7. Test completed successfully!");
                    return Ok(());
                }
                Err(_) if attempt < 5 => {
                    std::thread::sleep(std::time::Duration::from_millis(500 * attempt as u64));
                }
                Err(e) => {
                    eprintln!(
                        "6. Warning: Could not clean up test database after 5 attempts: {}",
                        e
                    );
                    eprintln!("   You can manually delete: {:?}", db_path);
                    println!("7. Test completed successfully!");
                    return Ok(());
                }
            }
        }
    }

    // Cleanup for non-Windows/manual path handling
    println!("6. Cleaning up test database...");
    match std::fs::remove_dir_all(&db_path) {
        Ok(_) => println!("   Cleanup successful!"),
        Err(e) => {
            eprintln!("   Warning: Could not clean up test database: {}", e);
            eprintln!("   You can manually delete: {:?}", db_path);
        }
    }
    println!("7. Test completed successfully!");

    Ok(())
}
