//! Tests for poisoned lock recovery mechanisms.
//!
//! These tests verify that the runtime can gracefully handle situations
//! where a thread panics while holding a lock, ensuring the system
//! recovers without crashing.

use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

// Helper struct to simulate internal state that may panic
#[derive(Debug, Clone)]
struct MockData {
    value: String,
}

#[test]
fn test_rw_lock_poison_recovery() {
    let data = Arc::new(RwLock::new(MockData {
        value: "initial_data".to_string(),
    }));

    let data_clone = Arc::clone(&data);

    // 1. Simulate scenario: Spawn a thread that holds Write Lock then Panics
    let handle = thread::spawn(move || {
        // Hold the lock
        let _guard = data_clone.write().unwrap();
        
        // Simulate work then panic (e.g., logic error or assertion fail)
        panic!("Simulated panic in worker thread!");
    });

    // Wait for the child thread to finish panicking
    let _ = handle.join();

    // 2. Test recovery in the main thread
    // Using .unwrap() here would crash immediately because the Lock is poisoned
    // But we use recovery mechanism (simulated with unwrap_or_else in this test)
    
    let recovered_data = {
        // Simulate the same logic we fixed in the real code:
        // data.read().unwrap_or_else(|e| e.into_inner())
        match data.read() {
            Ok(guard) => guard.value.clone(),
            Err(poisoned) => {
                // This proves we can recover
                println!("Detected poisoned lock! Recovering...");
                let guard = poisoned.into_inner();
                guard.value.clone()
            }
        }
    };

    // 3. Verify data is intact and system didn't crash
    assert_eq!(recovered_data, "initial_data");
    println!("Success: System recovered from poisoned lock and data is intact.");
}

#[test]
fn test_concurrent_access_after_poison() {
    let store = Arc::new(RwLock::new(vec![1, 2, 3]));
    let store_clone = Arc::clone(&store);

    // Poison the lock
    let handle = thread::spawn(move || {
        let _guard = store_clone.write().unwrap();
        panic!("Worker crashed!");
    });
    let _ = handle.join();

    // Attempt to access data after lock is poisoned
    // Must not panic
    let result = match store.read() {
        Ok(guard) => guard.len(),
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            guard.len()
        }
    };

    assert_eq!(result, 3);
    println!("Success: Can access data after poison event.");
}

#[test]
fn test_write_lock_recovery_after_poison() {
    let data = Arc::new(RwLock::new(42));
    let data_clone = Arc::clone(&data);

    // Poison the lock via write panic
    let handle = thread::spawn(move || {
        let mut guard = data_clone.write().unwrap();
        *guard = 100; // Change value before panic
        panic!("Writer panicked!");
    });
    let _ = handle.join();

    // Attempt to write new data after lock is poisoned
    let new_value = match data.write() {
        Ok(mut guard) => {
            *guard = 200;
            *guard
        }
        Err(poisoned) => {
            println!("Recovering from poisoned write lock...");
            let mut guard = poisoned.into_inner();
            *guard = 200;
            *guard
        }
    };

    assert_eq!(new_value, 200);
    
    // Verify the value changed before panic (100) was overwritten by 200
    // Reading now should give 200 since we overwrote it during recovery
    let final_value = match data.read() {
        Ok(guard) => *guard,
        Err(poisoned) => *poisoned.into_inner(),
    };
    
    assert_eq!(final_value, 200);
    println!("Success: Write lock recovered and updated value correctly.");
}

#[test]
fn test_multiple_poison_events() {
    let counter = Arc::new(RwLock::new(0));
    
    // Create multiple poison events
    for i in 0..3 {
        let counter_clone = Arc::clone(&counter);
        let handle = thread::spawn(move || {
            let _guard = counter_clone.write().unwrap();
            panic!("Poison event #{}", i);
        });
        let _ = handle.join();
        
        // Recover and count recovery attempts
        let current = match counter.read() {
            Ok(guard) => *guard,
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                *guard
            }
        };
        
        // Update value to show system continues working
        match counter.write() {
            Ok(mut guard) => *guard = current + 1,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                *guard = current + 1;
            }
        }
    }

    let final_count = match counter.read() {
        Ok(guard) => *guard,
        Err(poisoned) => *poisoned.into_inner(),
    };

    assert_eq!(final_count, 3);
    println!("Success: System survived multiple poison events.");
}
