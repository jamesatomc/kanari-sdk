#[path = "gas_v1.rs"]
pub mod gas_v1;
#[path = "gas_v2.rs"]
pub mod gas_v2;

/// Select the active gas implementation here.
///
/// Change `select_gas_impl!(v1);` to `select_gas_impl!(v2);`
/// when you want to switch to the zero-fee gas model.
macro_rules! select_gas_impl {
    (v1) => {
        pub use self::gas_v1::*;
    };
    (v2) => {
        pub use self::gas_v2::*;
    };
}

select_gas_impl!(v1);
