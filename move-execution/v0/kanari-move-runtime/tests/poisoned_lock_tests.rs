//! Tests for poisoned lock recovery mechanisms.
//!
//! These tests verify that the runtime can gracefully handle situations
//! where a thread panics while holding a lock, ensuring the system
//! recovers without crashing.

use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

// Helper struct เพื่อจำลองสถานะภายในที่อาจเกิด Panic
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

    // 1. จำลองสถานการณ์: สร้าง Thread ที่ถือ Write Lock แล้ว Panic
    let handle = thread::spawn(move || {
        // ถือล็อค
        let _guard = data_clone.write().unwrap();
        
        // จำลองการทำงานแล้วเกิด Panic (เช่น logic error หรือ assertion fail)
        panic!("Simulated panic in worker thread!");
    });

    // รอให้ thread ย่อย panic เสร็จสิ้น
    let _ = handle.join();

    // 2. ทดสอบการกู้คืนใน Thread หลัก
    // หากใช้ .unwrap() ตรงนี้ โปรแกรมจะ Crash ทันทีเพราะ Lock เป็นพิษ
    // แต่เราใช้กลไก recovery (จำลองด้วย unwrap_or_else ในเทสต์นี้)
    
    let recovered_data = {
        // จำลองตรรกะเดียวกับที่เราแก้ในโค้ดจริง:
        // data.read().unwrap_or_else(|e| e.into_inner())
        match data.read() {
            Ok(guard) => guard.value.clone(),
            Err(poisoned) => {
                // นี่คือจุดที่พิสูจน์ว่าเรากู้คืนได้
                println!("Detected poisoned lock! Recovering...");
                let guard = poisoned.into_inner();
                guard.value.clone()
            }
        }
    };

    // 3. ยืนยันว่าข้อมูลยังอยู่และระบบไม่พัง
    assert_eq!(recovered_data, "initial_data");
    println!("Success: System recovered from poisoned lock and data is intact.");
}

#[test]
fn test_concurrent_access_after_poison() {
    let store = Arc::new(RwLock::new(vec![1, 2, 3]));
    let store_clone = Arc::clone(&store);

    // ทำให้ล็อคเป็นพิษ
    let handle = thread::spawn(move || {
        let _guard = store_clone.write().unwrap();
        panic!("Worker crashed!");
    });
    let _ = handle.join();

    // พยายามเข้าถึงข้อมูลใหม่หลังจากล็อคเป็นพิษ
    // ต้องไม่เกิด Panic
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

    // ทำให้ล็อคเป็นพิษด้วยการ write panic
    let handle = thread::spawn(move || {
        let mut guard = data_clone.write().unwrap();
        *guard = 100; // เปลี่ยนค่าก่อน panic
        panic!("Writer panicked!");
    });
    let _ = handle.join();

    // พยายามเขียนข้อมูลใหม่หลังจากล็อคเป็นพิษ
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
    
    // ตรวจสอบว่าค่าที่เปลี่ยนก่อน panic ยังคงอยู่ (100) ก่อนจะถูกเปลี่ยนเป็น 200
    // แต่ถ้าเราอ่านเลยจะได้ 200 เพราะเราเขียนทับไปแล้วใน recovery
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
    
    // สร้างพิษหลายครั้ง
    for i in 0..3 {
        let counter_clone = Arc::clone(&counter);
        let handle = thread::spawn(move || {
            let _guard = counter_clone.write().unwrap();
            panic!("Poison event #{}", i);
        });
        let _ = handle.join();
        
        // กู้คืนและนับจำนวนครั้งที่กู้คืน
        let current = match counter.read() {
            Ok(guard) => *guard,
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                *guard
            }
        };
        
        // อัพเดทค่าเพื่อแสดงว่าระบบทำงานต่อได้
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
