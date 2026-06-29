#![allow(clippy::print_stdout)]
// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Example demonstrating email-based authentication and transaction signing
//!
//! This example shows:
//! - User registration with email and password
//! - User login and session management
//! - Transaction signing using authenticated sessions
//! - Password change and account management

use anyhow::Result;
use kanari_auth::AuthManager;
use kanari_crypto::keys::CurveType;

fn main() -> Result<()> {
    println!("=== Kanari Email-Based Authentication System ===\n");

    // 1. Create authentication manager
    println!("1. Initializing authentication manager...");
    let mut auth = AuthManager::new();
    println!("   ✓ Auth manager created\n");

    // 2. Register a new user
    println!("2. Registering new user...");
    let email = "alice@example.com";
    let password = "SecurePass123!";

    let wallet = auth.register_user(email, password, Some(CurveType::Ed25519))?;
    println!("   Email: {}", email);
    println!("   Wallet Address: {}", wallet.address);
    println!("   Curve Type: {:?}", wallet.curve_type);
    println!("   ✓ User registered successfully\n");

    // 3. Login with credentials
    println!("3. Logging in...");
    let session = auth.login(email, password, None)?;
    println!("   Session ID: {}", session.session_id);
    println!("   Email: {}", session.email);
    println!("   Wallet: {}", session.wallet_address);
    if let Some(remaining) = session.time_remaining() {
        println!("   Session expires in: {:?}", remaining);
    }
    println!("   ✓ Login successful\n");

    // 4. Get user info from session
    println!("4. Retrieving user info from session...");
    let (user_email, wallet_addr) = auth.get_user_info(&session)?;
    println!("   Email: {}", user_email);
    println!("   Wallet: {}", wallet_addr);
    println!("   ✓ User info retrieved\n");

    // 5. Sign a transfer transaction
    println!("5. Signing a transfer transaction...");
    let recipient = "0x0000000000000000000000000000000000000000000000000000000000000456";
    let amount_mist = 1_000_000; // 1 KANARI token

    let signed_tx = auth.sign_transfer(
        &session,
        recipient,
        amount_mist,
        Some(100_000), // gas limit
        Some(1_000),   // gas price
    )?;

    println!("   Transaction Type: Transfer");
    println!("   From: {}", signed_tx.transaction.sender());
    println!("   To: {}", recipient);
    println!("   Amount: {} Mist", amount_mist);
    println!("   Gas Limit: {}", signed_tx.transaction.gas_limit());
    println!("   Gas Price: {} Mist", signed_tx.transaction.gas_price());
    println!(
        "   Sequence Number: {}",
        signed_tx.transaction.sequence_number()
    );
    println!("   Signature Length: {} bytes", signed_tx.signature.len());
    println!("   ✓ Transaction signed successfully\n");

    // 6. Register another user
    println!("6. Registering second user...");
    let bob_wallet =
        auth.register_user("bob@example.com", "BobPassword456!", Some(CurveType::K256))?;
    println!("   Email: bob@example.com");
    println!("   Wallet: {}", bob_wallet.address);
    println!("   ✓ Second user registered\n");

    // 7. List all users
    println!("7. Listing all registered users...");
    let users = auth.list_users();
    println!("   Total users: {}", users.len());
    for user in &users {
        println!("   - {}", user);
    }
    println!();

    // 8. Change password
    println!("8. Changing password for alice...");
    auth.change_password(email, password, "NewSecurePass789!")?;
    println!("   ✓ Password changed successfully");

    // Note: change_password automatically logs out all sessions
    println!("   Note: All sessions were invalidated by password change");

    // Old password should fail
    println!("   Testing old password (should fail)...");
    match auth.login(email, password, None) {
        Ok(_) => println!("   ✗ Unexpected: old password still works"),
        Err(_) => println!("   ✓ Old password correctly rejected"),
    }

    // New password should work
    println!("   Testing new password...");
    let new_session = auth.login(email, "NewSecurePass789!", None)?;
    println!(
        "   ✓ New password works, session: {}\n",
        new_session.session_id
    );

    // 9. Logout - use the new_session since old one was invalidated
    println!("9. Logging out...");
    auth.logout(&new_session.session_id)?;
    println!("   ✓ Session invalidated");

    // Try to use expired session
    println!("   Testing invalidated session (should fail)...");
    match auth.get_user_info(&session) {
        Ok(_) => println!("   ✗ Unexpected: invalidated session still works"),
        Err(_) => println!("   ✓ Invalidated session correctly rejected\n"),
    }

    // 10. Logout all sessions for a user
    println!("10. Logging out all sessions for alice...");
    auth.logout_all(email)?;
    println!("    ✓ All sessions invalidated\n");

    // 11. Delete account
    println!("11. Deleting bob's account...");
    auth.delete_account("bob@example.com", "BobPassword456!")?;
    println!("    ✓ Account deleted");

    let users = auth.list_users();
    println!("    Remaining users: {}", users.len());
    for user in &users {
        println!("    - {}", user);
    }
    println!();

    println!("=== Example completed successfully! ===");
    Ok(())
}
