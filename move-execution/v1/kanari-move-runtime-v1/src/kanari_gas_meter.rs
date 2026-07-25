use move_binary_format::errors::{PartialVMError, PartialVMResult};
use move_core_types::gas_algebra::InternalGas;
use move_core_types::vm_status::StatusCode;
use move_vm_types::gas::GasMeter;

/// Weighted gas meter used to cap VM execution work.
/// Coin deduction is handled outside this type; this meter only tracks internal execution cost.
pub(crate) struct KanariGasMeter {
    /// Internal gas consumed so far.
    gas_used: u64,
    /// Maximum internal gas allowed for one execution.
    gas_limit: u64,
}

impl KanariGasMeter {
    pub(crate) fn new(gas_limit: u64) -> Self {
        Self {
            gas_used: 0,
            gas_limit,
        }
    }

    pub(crate) fn gas_used(&self) -> u64 {
        self.gas_used
    }

    /// Charge additional internal gas and fail once the limit is exceeded.
    #[inline]
    fn charge(&mut self, amount: u64) -> PartialVMResult<()> {
        self.gas_used = self.gas_used.checked_add(amount).ok_or_else(|| {
            out_of_gas_error("Kanari execution gas counter overflowed".to_string())
        })?;

        if self.gas_used > self.gas_limit {
            return Err(out_of_gas_error(
                "Kanari execution limit exceeded".to_string(),
            ));
        }
        Ok(())
    }

    #[inline]
    fn charge_internal_gas(&mut self, amount: InternalGas) -> PartialVMResult<()> {
        self.charge(amount.into())
    }

    #[inline]
    fn charge_with_len(&mut self, base: u64, len: usize) -> PartialVMResult<()> {
        self.charge(base.saturating_add(len as u64))
    }

    #[inline]
    fn generic_extra(is_generic: bool, extra: u64) -> u64 {
        if is_generic { extra } else { 0 }
    }

    #[inline]
    fn value_size(value: &impl move_vm_types::views::ValueView) -> u64 {
        value.legacy_abstract_memory_size().into()
    }
}

fn out_of_gas_error(message: String) -> PartialVMError {
    PartialVMError::new(StatusCode::OUT_OF_GAS).with_message(message)
}

const SIMPLE_INSTR_COST: u64 = 1;
const CONST_LOAD_COST: u64 = 2;
const COPY_LOC_COST: u64 = 2;
const MOVE_LOC_COST: u64 = 2;
const STORE_LOC_COST: u64 = 3;
const CALL_COST: u64 = 8;
const CALL_GENERIC_COST: u64 = 10;
const PACK_COST: u64 = 5;
const PACK_GENERIC_EXTRA_COST: u64 = 2;
const UNPACK_COST: u64 = 5;
const UNPACK_GENERIC_EXTRA_COST: u64 = 2;
const READ_REF_COST: u64 = 3;
const WRITE_REF_COST: u64 = 6;
const EQ_COST: u64 = 2;
const NEQ_COST: u64 = 2;
const VEC_PACK_COST: u64 = 4;
const VEC_LEN_COST: u64 = 2;
const VEC_BORROW_COST: u64 = 3;
const VEC_PUSH_BACK_COST: u64 = 4;
const VEC_POP_BACK_COST: u64 = 4;
const VEC_UNPACK_COST: u64 = 6;
const VEC_SWAP_COST: u64 = 4;
const NATIVE_FUNCTION_PRE_EXEC_COST: u64 = 8;
const DROP_FRAME_COST: u64 = 3;

// `GasMeter` implementation backed by the lightweight counter above.

impl GasMeter for KanariGasMeter {
    fn remaining_gas(&self) -> InternalGas {
        let remaining = self.gas_limit.saturating_sub(self.gas_used);
        InternalGas::new(remaining)
    }

    fn charge_simple_instr(
        &mut self,
        _instr: move_vm_types::gas::SimpleInstruction,
    ) -> PartialVMResult<()> {
        self.charge(SIMPLE_INSTR_COST)
    }

    fn charge_pop(
        &mut self,
        popped_val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(SIMPLE_INSTR_COST.saturating_add(Self::value_size(&popped_val)))
    }

    fn charge_call(
        &mut self,
        _module_id: &move_core_types::language_storage::ModuleId,
        _func_name: &str,
        args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
        num_locals: move_core_types::gas_algebra::NumArgs,
    ) -> PartialVMResult<()> {
        self.charge(
            CALL_COST
                .saturating_add(args.len() as u64)
                .saturating_add(u64::from(num_locals)),
        )
    }

    fn charge_call_generic(
        &mut self,
        _module_id: &move_core_types::language_storage::ModuleId,
        _func_name: &str,
        ty_args: impl ExactSizeIterator<Item = impl move_vm_types::views::TypeView>,
        args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
        num_locals: move_core_types::gas_algebra::NumArgs,
    ) -> PartialVMResult<()> {
        self.charge(
            CALL_GENERIC_COST
                .saturating_add(ty_args.len() as u64)
                .saturating_add(args.len() as u64)
                .saturating_add(u64::from(num_locals)),
        )
    }

    fn charge_ld_const(
        &mut self,
        size: move_core_types::gas_algebra::NumBytes,
    ) -> PartialVMResult<()> {
        self.charge(CONST_LOAD_COST.saturating_add(u64::from(size)))
    }

    fn charge_ld_const_after_deserialization(
        &mut self,
        val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(CONST_LOAD_COST.saturating_add(Self::value_size(&val)))
    }

    fn charge_copy_loc(
        &mut self,
        val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(COPY_LOC_COST.saturating_add(Self::value_size(&val)))
    }

    fn charge_move_loc(
        &mut self,
        _val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(MOVE_LOC_COST)
    }

    fn charge_store_loc(
        &mut self,
        val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(STORE_LOC_COST.saturating_add(Self::value_size(&val)))
    }

    fn charge_pack(
        &mut self,
        is_generic: bool,
        args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge_with_len(
            PACK_COST.saturating_add(Self::generic_extra(is_generic, PACK_GENERIC_EXTRA_COST)),
            args.len(),
        )
    }

    fn charge_unpack(
        &mut self,
        is_generic: bool,
        args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge_with_len(
            UNPACK_COST.saturating_add(Self::generic_extra(is_generic, UNPACK_GENERIC_EXTRA_COST)),
            args.len(),
        )
    }

    fn charge_read_ref(
        &mut self,
        val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(READ_REF_COST.saturating_add(Self::value_size(&val)))
    }

    fn charge_write_ref(
        &mut self,
        new_val: impl move_vm_types::views::ValueView,
        old_val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(
            WRITE_REF_COST
                .saturating_add(Self::value_size(&new_val))
                .saturating_add(Self::value_size(&old_val)),
        )
    }

    fn charge_eq(
        &mut self,
        lhs: impl move_vm_types::views::ValueView,
        rhs: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(
            EQ_COST
                .saturating_add(Self::value_size(&lhs))
                .saturating_add(Self::value_size(&rhs)),
        )
    }

    fn charge_neq(
        &mut self,
        lhs: impl move_vm_types::views::ValueView,
        rhs: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(
            NEQ_COST
                .saturating_add(Self::value_size(&lhs))
                .saturating_add(Self::value_size(&rhs)),
        )
    }

    fn charge_vec_pack<'a>(
        &mut self,
        _ty: impl move_vm_types::views::TypeView + 'a,
        args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge_with_len(VEC_PACK_COST, args.len())
    }

    fn charge_vec_len(&mut self, _ty: impl move_vm_types::views::TypeView) -> PartialVMResult<()> {
        self.charge(VEC_LEN_COST)
    }

    fn charge_vec_borrow(
        &mut self,
        _is_mut: bool,
        _ty: impl move_vm_types::views::TypeView,
        _is_success: bool,
    ) -> PartialVMResult<()> {
        self.charge(VEC_BORROW_COST)
    }

    fn charge_vec_push_back(
        &mut self,
        _ty: impl move_vm_types::views::TypeView,
        _val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(VEC_PUSH_BACK_COST)
    }

    fn charge_vec_pop_back(
        &mut self,
        _ty: impl move_vm_types::views::TypeView,
        _val: Option<impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge(VEC_POP_BACK_COST)
    }

    fn charge_vec_unpack(
        &mut self,
        _ty: impl move_vm_types::views::TypeView,
        expect_num_elements: move_core_types::gas_algebra::NumArgs,
        _elems: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge(VEC_UNPACK_COST.saturating_add(u64::from(expect_num_elements)))
    }

    fn charge_vec_swap(&mut self, _ty: impl move_vm_types::views::TypeView) -> PartialVMResult<()> {
        self.charge(VEC_SWAP_COST)
    }

    fn charge_native_function(
        &mut self,
        amount: InternalGas,
        _ret_vals: Option<impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>>,
    ) -> PartialVMResult<()> {
        self.charge_internal_gas(amount)
    }

    fn charge_native_function_before_execution(
        &mut self,
        ty_args: impl ExactSizeIterator<Item = impl move_vm_types::views::TypeView>,
        args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge(
            NATIVE_FUNCTION_PRE_EXEC_COST
                .saturating_add(ty_args.len() as u64)
                .saturating_add(args.len() as u64),
        )
    }

    fn charge_drop_frame(
        &mut self,
        _locals: impl Iterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge(DROP_FRAME_COST)
    }

    fn get_profiler_mut(&mut self) -> Option<&mut move_vm_profiler::GasProfiler> {
        None
    }

    fn set_profiler(&mut self, _profiler: move_vm_profiler::GasProfiler) {
        // Do nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanari_types::error::KanariUnwrapExt;
    use move_core_types::gas_algebra::NumBytes;
    use move_vm_types::gas::GasMeter;

    #[test]
    fn native_function_uses_reported_internal_gas() {
        let mut meter = KanariGasMeter::new(10);
        let ret_vals: Option<std::iter::Empty<move_vm_types::values::Value>> = None;
        let err = meter
            .charge_native_function(InternalGas::new(11), ret_vals)
            .expect_err("native gas should count toward the execution cap");
        assert_eq!(err.major_status(), StatusCode::OUT_OF_GAS);
    }

    #[test]
    fn gas_counter_overflow_becomes_out_of_gas() {
        let mut meter = KanariGasMeter::new(u64::MAX);
        meter
            .charge(u64::MAX)
            .invariant("max charge should fit once");

        let err = meter
            .charge(1)
            .expect_err("overflow should not silently disable the gas cap");
        assert_eq!(err.major_status(), StatusCode::OUT_OF_GAS);
    }

    #[test]
    fn ld_const_scales_with_constant_size() {
        let mut meter = KanariGasMeter::new(5);
        let err = meter
            .charge_ld_const(NumBytes::new(4))
            .expect_err("large constants should consume more gas than tiny ones");
        assert_eq!(err.major_status(), StatusCode::OUT_OF_GAS);
    }

    #[test]
    fn copying_large_values_scales_with_value_size() {
        let mut meter = KanariGasMeter::new(16);
        let value = move_vm_types::values::Value::vector_u8(vec![0; 64]);
        let err = meter
            .charge_copy_loc(value)
            .expect_err("copying a large value must consume size-dependent gas");
        assert_eq!(err.major_status(), StatusCode::OUT_OF_GAS);
    }

    #[test]
    fn popped_values_scale_with_value_size() {
        let mut meter = KanariGasMeter::new(16);
        let value = move_vm_types::values::Value::vector_u8(vec![0; 64]);
        let err = meter
            .charge_pop(value)
            .expect_err("popping a large value must consume size-dependent gas");
        assert_eq!(err.major_status(), StatusCode::OUT_OF_GAS);
    }

    #[test]
    fn deserialized_constants_scale_with_value_size() {
        let mut meter = KanariGasMeter::new(16);
        let value = move_vm_types::values::Value::vector_u8(vec![0; 64]);
        let err = meter
            .charge_ld_const_after_deserialization(value)
            .expect_err("deserialized large constants must consume size-dependent gas");
        assert_eq!(err.major_status(), StatusCode::OUT_OF_GAS);
    }

    #[test]
    fn generic_pack_and_unpack_charge_extra() {
        let mut plain_pack = KanariGasMeter::new(PACK_COST);
        plain_pack
            .charge_pack(false, std::iter::empty::<move_vm_types::values::Value>())
            .expect("plain pack should fit the base cost");

        let mut generic_pack = KanariGasMeter::new(PACK_COST);
        let err = generic_pack
            .charge_pack(true, std::iter::empty::<move_vm_types::values::Value>())
            .expect_err("generic pack should charge more than plain pack");
        assert_eq!(err.major_status(), StatusCode::OUT_OF_GAS);

        let mut plain_unpack = KanariGasMeter::new(UNPACK_COST);
        plain_unpack
            .charge_unpack(false, std::iter::empty::<move_vm_types::values::Value>())
            .expect("plain unpack should fit the base cost");

        let mut generic_unpack = KanariGasMeter::new(UNPACK_COST);
        let err = generic_unpack
            .charge_unpack(true, std::iter::empty::<move_vm_types::values::Value>())
            .expect_err("generic unpack should charge more than plain unpack");
        assert_eq!(err.major_status(), StatusCode::OUT_OF_GAS);
    }
}
