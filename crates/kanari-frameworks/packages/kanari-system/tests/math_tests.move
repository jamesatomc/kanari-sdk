// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#[test_only]
module kanari_system::math_tests {
    use kanari_system::math;

    // =================================================================
    // Tests: Min / Max
    // =================================================================
    #[test]
    fun test_min_max_u64() {
        assert!(math::min_u64(10, 20) == 10, 0);
        assert!(math::min_u64(20, 10) == 10, 1);
        assert!(math::min_u64(15, 15) == 15, 2);

        assert!(math::max_u64(10, 20) == 20, 3);
        assert!(math::max_u64(20, 10) == 20, 4);
        assert!(math::max_u64(15, 15) == 15, 5);
    }

    // =================================================================
    // Tests: Rounding (หัวใจสำคัญของ DeFi ป้องกัน Pool ขาดทุน)
    // =================================================================
    #[test]
    fun test_divide_and_round_up() {
        // หารลงตัว ต้องไม่ปัดเพิ่ม
        assert!(math::divide_and_round_up(10, 2) == 5, 0); 
        assert!(math::divide_and_round_up(9, 3) == 3, 1);

        // หารไม่ลงตัว ต้องปัดขึ้นเสมอ
        assert!(math::divide_and_round_up(10, 3) == 4, 2); // 3.333 -> 4
        assert!(math::divide_and_round_up(11, 3) == 4, 3); // 3.666 -> 4
        assert!(math::divide_and_round_up(1, 100) == 1, 4); // 0.01 -> 1
    }

    #[test]
    #[expected_failure(location = kanari_system::math, abort_code = math::E_DIVIDE_BY_ZERO)]
    fun test_divide_and_round_up_zero_denominator() {
        math::divide_and_round_up(10, 0);
    }

    // =================================================================
    // Tests: Native Functions (ทศสอบว่า Rust VM รันคณิตศาสตร์ผ่านไหม)
    // =================================================================
    #[test]
    fun test_native_sqrt() {
        assert!(math::sqrt_u64(100) == 10, 0);
        assert!(math::sqrt_u64(144) == 12, 1);
        assert!(math::sqrt_u64(2) == 1, 2); // รูท 2 ปัดเศษลงเหลือ 1
    }

    #[test]
    fun test_native_pow() {
        assert!(math::pow_u64(2, 3) == 8, 0);
        assert!(math::pow_u64(10, 4) == 10000, 1);
        assert!(math::pow_u64(5, 0) == 1, 2); // ยกกำลัง 0 ต้องได้ 1
    }

    #[test]
    fun test_native_mul_div() {
        // (10 * 20) / 4 = 50
        assert!(math::mul_div_u64(10, 20, 4) == 50, 0);
    }

    #[test]
    fun test_u128_min_max_and_diff() {
        assert!(math::min_u128(10, 20) == 10, 10);
        assert!(math::max_u128(10, 20) == 20, 11);
        assert!(math::diff_u128(20, 7) == 13, 12);
        assert!(math::diff_u128(7, 20) == 13, 13);
        assert!(math::diff_u128(9, 9) == 0, 14);
    }

    #[test]
    fun test_mul_div_round_up_u64() {
        assert!(math::mul_div_round_up_u64(10, 20, 4) == 50, 20);
        assert!(math::mul_div_round_up_u64(10, 20, 3) == 67, 21);
        assert!(math::mul_div_round_up_u64(0, 20, 3) == 0, 22);
    }

    #[test]
    fun test_divide_and_round_up_u128() {
        assert!(math::divide_and_round_up_u128(10, 2) == 5, 30);
        assert!(math::divide_and_round_up_u128(10, 3) == 4, 31);
        assert!(math::divide_and_round_up_u128(0, 3) == 0, 32);
    }

    #[test]
    fun test_diff_u64() {
        assert!(math::diff_u64(20, 7) == 13, 40);
        assert!(math::diff_u64(7, 20) == 13, 41);
        assert!(math::diff_u64(9, 9) == 0, 42);
    }

    #[test]
    #[expected_failure(location = kanari_system::math, abort_code = math::E_DIVIDE_BY_ZERO)]
    fun test_mul_div_round_up_zero_denominator() {
        math::mul_div_round_up_u64(10, 20, 0);
    }

    #[test]
    #[expected_failure(location = kanari_system::math, abort_code = math::E_DIVIDE_BY_ZERO)]
    fun test_mul_div_u64_zero_denominator() {
        math::mul_div_u64(10, 20, 0);
    }

    #[test]
    #[expected_failure(location = kanari_system::math, abort_code = math::E_RESULT_OVERFLOW)]
    fun test_mul_div_u64_result_overflow() {
        // (U64MAX * U64MAX) / 1 does not fit in u64 — must abort, not truncate.
        math::mul_div_u64(18446744073709551615, 18446744073709551615, 1);
    }

    #[test]
    #[expected_failure(location = kanari_system::math, abort_code = math::E_RESULT_OVERFLOW)]
    fun test_mul_div_round_up_u64_result_overflow() {
        math::mul_div_round_up_u64(18446744073709551615, 18446744073709551615, 1);
    }
}