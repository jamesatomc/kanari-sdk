// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::math {
    
    // =================================================================
    // Error Codes
    // (These errors will be thrown (aborted) from the Native Rust side)
    // =================================================================
    #[allow(unused_const)]
    const E_OVERFLOW: u64 = 1;
    const E_DIVIDE_BY_ZERO: u64 = 2;

    // =================================================================
    // Native Functions (Processed in Rust: Fast & Gas Efficient)
    // =================================================================

    /// Calculate the square root of a u128
    public native fun sqrt_u128(x: u128): u128;

    /// Calculate the square root of a u64
    public native fun sqrt_u64(x: u64): u64;

    /// Calculate power (base ^ exponent)
    public native fun pow_u64(base: u64, exponent: u8): u64;

    /// Safely calculate (x * y) / z for u128
    /// Most commonly used in AMMs to prevent overflow issues during multiplication
    public native fun mul_div_u128(x: u128, y: u128, z: u128): u128;


    // =================================================================
    // Move Functions (Convenience functions on the Move VM)
    // =================================================================

    const E_RESULT_OVERFLOW: u64 = 3;
    const U64_MAX_U128: u128 = 18446744073709551615;

    /// Helper: Calculate (x * y) / z for u64
    /// Converts to u128 for Native calculation to prevent Overflow, then converts back to u64.
    /// SECURITY: aborts on `z == 0` and when the 128-bit result does not fit in u64
    /// (previously silently truncated via `as u64`).
    public fun mul_div_u64(x: u64, y: u64, z: u64): u64 {
        assert!(z > 0, E_DIVIDE_BY_ZERO);
        let result = mul_div_u128((x as u128), (y as u128), (z as u128));
        assert!(result <= U64_MAX_U128, E_RESULT_OVERFLOW);
        (result as u64)
    }

    // --- Min / Max Utilities ---

    /// Calculate the minimum value for u128
    public fun min_u128(x: u128, y: u128): u128 {
        if (x < y) { x } else { y }
    }

    /// Calculate the maximum value for u128
    public fun max_u128(x: u128, y: u128): u128 {
        if (x > y) { x } else { y }
    }

    /// Calculate the minimum value for u64 (Commonly used in DEXs when comparing Liquidity)
    public fun min_u64(x: u64, y: u64): u64 {
        if (x < y) { x } else { y }
    }

    /// Calculate the maximum value for u64
    public fun max_u64(x: u64, y: u64): u64 {
        if (x > y) { x } else { y }
    }

    // =================================================================
    // Advanced DeFi Security (Preventing Pool Loss)
    // =================================================================

    /// Multiply and divide with round up for u64
    /// (Commonly used when calculating the Amount In a user must pay into the Pool to get the desired Amount Out)
    /// SECURITY: aborts on overflow of the rounded result (previously wrapped).
    public fun mul_div_round_up_u64(x: u64, y: u64, z: u64): u64 {
        let x_128 = (x as u128);
        let y_128 = (y as u128);
        let z_128 = (z as u128);
        assert!(z_128 > 0, E_DIVIDE_BY_ZERO);

        let prod = x_128 * y_128;
        let res = prod / z_128;

        // If there is a remainder, immediately round up by +1
        if (prod % z_128 == 0) {
            assert!(res <= U64_MAX_U128, E_RESULT_OVERFLOW);
            (res as u64)
        } else {
            assert!(res + 1 <= U64_MAX_U128, E_RESULT_OVERFLOW);
            ((res + 1) as u64)
        }
    }

    /// Calculate the absolute difference for u128 (Paired with u64)
    public fun diff_u128(x: u128, y: u128): u128 {
        if (x > y) { x - y } else { y - x }
    }

    // =================================================================
    // DeFi Math Utilities (Rounding & Additional Calculations)
    // =================================================================

    /// Divide and round up
    /// Used when calculating the number of coins a user must pay into the Pool 
    /// to prevent the Pool from losing value due to fractional amounts (dust)
    public fun divide_and_round_up(x: u64, y: u64): u64 {
        assert!(y > 0, E_DIVIDE_BY_ZERO);
        if (x == 0) {
            0
        } else {
            // Popular formula: (x - 1) / y + 1
            ((x - 1) / y) + 1
        }
    }

    /// Divide and round up for u128
    public fun divide_and_round_up_u128(x: u128, y: u128): u128 {
        assert!(y > 0, E_DIVIDE_BY_ZERO);
        if (x == 0) {
            0
        } else {
            ((x - 1) / y) + 1
        }
    }

    /// Calculate the absolute difference
    /// Frequently used to check if price fluctuation (Slippage) exceeds a specified limit
    public fun diff_u64(x: u64, y: u64): u64 {
        if (x > y) { x - y } else { y - x }
    }
}