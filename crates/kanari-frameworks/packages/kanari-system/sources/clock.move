// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

/// APIs for accessing time from move calls, via the `Clock`: a unique
/// shared object that is created during genesis.
module kanari_system::clock {
    use kanari_system::object::{Self, UID};
    use kanari_system::tx_context::{Self, TxContext};
    use kanari_system::transfer;

    /// Sender is not @0x0 the system address.
    const E_NOT_SYSTEM_ADDRESS: u64 = 0;

    /// Timestamp is not monotonic (not greater than or equal to current time)
    const E_TIMESTAMP_NOT_MONOTONIC: u64 = 1;

    /// Singleton shared object that exposes time to Move calls.
    struct Clock has key, store {
        id: UID,
        timestamp_ms: u64,
    }

    /// The `clock`'s current timestamp as a running total of
    /// milliseconds since an arbitrary point in the past.
    public fun timestamp_ms(clock: &Clock): u64 {
        clock.timestamp_ms
    }

    /// Create and share the singleton Clock -- this function is
    /// called exactly once, during genesis.
    public fun create(ctx: &mut TxContext) {
        let sender = tx_context::sender(ctx);
        assert!(sender == @0x0 || sender == @0x2, E_NOT_SYSTEM_ADDRESS);

        let clock = Clock {
            id: object::new(ctx), 
            timestamp_ms: 0,
        };

        object::save_object(&clock); 
        transfer::public_transfer_to_system(clock);
    }

    /// System call: Validator (the Rust node) will call this function every time the block is closed.
    public fun consensus_commit_prologue(clock: &mut Clock, timestamp_ms: u64, ctx: &TxContext) {
        let sender = tx_context::sender(ctx);
        assert!(sender == @0x0 || sender == @0x2, E_NOT_SYSTEM_ADDRESS);
        assert!(timestamp_ms >= clock.timestamp_ms, E_TIMESTAMP_NOT_MONOTONIC);
        clock.timestamp_ms = timestamp_ms;
        object::save_object(clock);
    }

    // =================================================================
    // Functions for Testing
    // =================================================================

    #[test_only]
    public fun create_for_testing(ctx: &mut TxContext): Clock {
        Clock {
            id: object::new(ctx),
            timestamp_ms: 0,
        }
    }

    #[test_only]
    public fun increment_for_testing(clock: &mut Clock, tick: u64) {
        clock.timestamp_ms = clock.timestamp_ms + tick;
    }

    #[test_only]
    public fun set_for_testing(clock: &mut Clock, timestamp_ms: u64) {
        assert!(timestamp_ms >= clock.timestamp_ms, 1);
        clock.timestamp_ms = timestamp_ms;
    }

    #[test_only]
    public fun destroy_for_testing(clock: Clock) {
        let Clock { id, timestamp_ms: _ } = clock;
        object::delete(id);
    }
}