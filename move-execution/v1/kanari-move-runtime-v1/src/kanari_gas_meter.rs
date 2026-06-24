use move_binary_format::errors::{PartialVMError, PartialVMResult};
use move_core_types::gas_algebra::InternalGas;
use move_core_types::vm_status::StatusCode;
use move_vm_types::gas::GasMeter;

/// Weighted gas meter used to cap VM execution work.
/// Coin deduction is handled outside this type; this meter only tracks internal execution cost.
pub struct KanariGasMeter {
    /// Internal gas consumed so far.
    gas_used: u64,
    /// Maximum internal gas allowed for one execution.
    gas_limit: u64,
}

impl KanariGasMeter {
    pub fn new(gas_limit: u64) -> Self {
        Self {
            gas_used: 0,
            gas_limit,
        }
    }

    pub fn gas_used(&self) -> u64 {
        self.gas_used
    }

    /// Charge additional internal gas and fail once the limit is exceeded.
    #[inline]
    pub fn charge(&mut self, amount: u64) -> PartialVMResult<()> {
        self.gas_used = self.gas_used.saturating_add(amount);

        if self.gas_used > self.gas_limit {
            return Err(PartialVMError::new(StatusCode::OUT_OF_GAS).with_message(
                "Kanari Execution Limit Exceeded: Infinite Loop Detected!".to_string(),
            ));
        }
        Ok(())
    }
}

const SIMPLE_INSTR_COST: u64 = 1;
const CONST_LOAD_COST: u64 = 2;
const COPY_LOC_COST: u64 = 2;
const MOVE_LOC_COST: u64 = 2;
const STORE_LOC_COST: u64 = 3;
const CALL_COST: u64 = 8;
const CALL_GENERIC_COST: u64 = 10;
const PACK_COST: u64 = 5;
const UNPACK_COST: u64 = 5;
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
const NATIVE_FUNCTION_BASE_COST: u64 = 20;
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
        _popped_val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(SIMPLE_INSTR_COST)
    }

    fn charge_call(
        &mut self,
        _module_id: &move_core_types::language_storage::ModuleId,
        _func_name: &str,
        _args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
        _num_locals: move_core_types::gas_algebra::NumArgs,
    ) -> PartialVMResult<()> {
        self.charge(CALL_COST)
    }

    fn charge_call_generic(
        &mut self,
        _module_id: &move_core_types::language_storage::ModuleId,
        _func_name: &str,
        _ty_args: impl ExactSizeIterator<Item = impl move_vm_types::views::TypeView>,
        _args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
        _num_locals: move_core_types::gas_algebra::NumArgs,
    ) -> PartialVMResult<()> {
        self.charge(CALL_GENERIC_COST)
    }

    fn charge_ld_const(
        &mut self,
        _size: move_core_types::gas_algebra::NumBytes,
    ) -> PartialVMResult<()> {
        self.charge(CONST_LOAD_COST)
    }

    fn charge_ld_const_after_deserialization(
        &mut self,
        _val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(CONST_LOAD_COST)
    }

    fn charge_copy_loc(
        &mut self,
        _val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(COPY_LOC_COST)
    }

    fn charge_move_loc(
        &mut self,
        _val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(MOVE_LOC_COST)
    }

    fn charge_store_loc(
        &mut self,
        _val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(STORE_LOC_COST)
    }

    fn charge_pack(
        &mut self,
        _is_generic: bool,
        _args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge(PACK_COST)
    }

    fn charge_unpack(
        &mut self,
        _is_generic: bool,
        _args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge(UNPACK_COST)
    }

    fn charge_read_ref(
        &mut self,
        _val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(READ_REF_COST)
    }

    fn charge_write_ref(
        &mut self,
        _new_val: impl move_vm_types::views::ValueView,
        _old_val: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(WRITE_REF_COST)
    }

    fn charge_eq(
        &mut self,
        _lhs: impl move_vm_types::views::ValueView,
        _rhs: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(EQ_COST)
    }

    fn charge_neq(
        &mut self,
        _lhs: impl move_vm_types::views::ValueView,
        _rhs: impl move_vm_types::views::ValueView,
    ) -> PartialVMResult<()> {
        self.charge(NEQ_COST)
    }

    fn charge_vec_pack<'a>(
        &mut self,
        _ty: impl move_vm_types::views::TypeView + 'a,
        _args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge(VEC_PACK_COST)
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
        _expect_num_elements: move_core_types::gas_algebra::NumArgs,
        _elems: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge(VEC_UNPACK_COST)
    }

    fn charge_vec_swap(&mut self, _ty: impl move_vm_types::views::TypeView) -> PartialVMResult<()> {
        self.charge(VEC_SWAP_COST)
    }

    fn charge_native_function(
        &mut self,
        _amount: InternalGas,
        _ret_vals: Option<impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>>,
    ) -> PartialVMResult<()> {
        self.charge(NATIVE_FUNCTION_BASE_COST)
    }

    fn charge_native_function_before_execution(
        &mut self,
        _ty_args: impl ExactSizeIterator<Item = impl move_vm_types::views::TypeView>,
        _args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge(NATIVE_FUNCTION_PRE_EXEC_COST)
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
