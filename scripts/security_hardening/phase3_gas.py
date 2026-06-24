from __future__ import annotations

import re

from .common import insert_after_once, read, write


def apply() -> None:
    path = "crates/kanari-core/src/engine.rs"
    text = read(path)
    text = text.replace("const MAX_MEMPOOL_SIZE: usize = 1_000_000;", "const MAX_MEMPOOL_SIZE: usize = 50_000;", 1)
    text = text.replace(
        '''fn parse_type_tag(s: &str) -> Option<TypeTag> {
    fn split_top_level_commas''',
        '''fn parse_type_tag(s: &str) -> Option<TypeTag> {
    if s.len() > 4096 {
        return None;
    }
    let mut nesting = 0usize;
    for byte in s.bytes() {
        match byte {
            b'<' => {
                nesting = nesting.saturating_add(1);
                if nesting > 16 {
                    return None;
                }
            }
            b'>' => nesting = nesting.saturating_sub(1),
            _ => {}
        }
    }

    fn split_top_level_commas''',
        1,
    )
    text = text.replace(
        '''        gas_meter.consume(gas_op.gas_units())?;
        let gas_cost = gas_meter.total_cost();
        let total_required = required_amount.saturating_add(gas_cost);''',
        '''        gas_meter.consume(gas_op.gas_units())?;
        let base_gas_used = gas_meter.gas_used;
        let reserved_gas_cost = tx
            .gas_limit()
            .checked_mul(tx.gas_price())
            .ok_or_else(|| anyhow::anyhow!("Gas cost overflow"))?;
        let gas_cost = base_gas_used
            .checked_mul(tx.gas_price())
            .ok_or_else(|| anyhow::anyhow!("Gas cost overflow"))?;
        let total_required = required_amount.saturating_add(reserved_gas_cost);''',
        1,
    )
    text = text.replace(
        '''                    None,
                    timestamp,
                    Some(tx.hash()),
                    persist_runtime_state,''',
        '''                    Some((tx.gas_limit(), tx.gas_price())),
                    timestamp,
                    Some(tx.hash()),
                    persist_runtime_state,''',
        1,
    )
    text = text.replace(
        '''                    Some(sender_addr),
                    None,
                    timestamp,
                    Some(tx.hash()),''',
        '''                    Some(sender_addr),
                    Some((tx.gas_limit(), tx.gas_price())),
                    timestamp,
                    Some(tx.hash()),''',
        1,
    )
    text = text.replace(
        '''                    Err(e) => {
                        changeset.mark_failed(format!("Publish failed: {}", e));
                    }''',
        '''                    Err(e) => {
                        changeset.mark_failed(format!("Publish failed: {}", e));
                        changeset.set_gas_used(tx.gas_limit().saturating_sub(base_gas_used));
                    }''',
        1,
    )
    text = text.replace(
        '''                    Err(e) => {
                        changeset.mark_failed(format!("Execution failed: {}", e));
                    }''',
        '''                    Err(e) => {
                        changeset.mark_failed(format!("Execution failed: {}", e));
                        changeset.set_gas_used(tx.gas_limit().saturating_sub(base_gas_used));
                    }''',
        1,
    )
    old = '''        Self::apply_gas_and_sequence(&mut changeset, sender_addr, gas_cost, gas_meter.gas_used)?;
        Ok(changeset)'''
    new = '''        let vm_gas_used = changeset.gas_used;
        let actual_gas_used = base_gas_used.saturating_add(vm_gas_used).min(tx.gas_limit());
        let actual_gas_cost = actual_gas_used
            .checked_mul(tx.gas_price())
            .ok_or_else(|| anyhow::anyhow!("Gas cost overflow"))?;
        Self::apply_gas_and_sequence(&mut changeset, sender_addr, actual_gas_cost, actual_gas_used)?;
        Ok(changeset)'''
    if old not in text:
        raise RuntimeError("engine final gas accounting block not found")
    text = text.replace(old, new, 1)
    write(path, text)

    meter = "move-execution/v1/kanari-move-runtime-v1/src/kanari_gas_meter.rs"
    insert_after_once(
        meter,
        '''    pub fn new(gas_limit: u64) -> Self {
        Self {
            gas_used: 0,
            gas_limit,
        }
    }
''',
        '''
    pub fn gas_used(&self) -> u64 {
        self.gas_used
    }
''',
    )
    text = read(meter)
    text = text.replace(
        '''        _amount: InternalGas,
        _ret_vals: Option<impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>>,
    ) -> PartialVMResult<()> {
        self.charge(NATIVE_FUNCTION_BASE_COST)''',
        '''        amount: InternalGas,
        ret_vals: Option<impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>>,
    ) -> PartialVMResult<()> {
        let return_cost = ret_vals.map(|values| values.len() as u64).unwrap_or(0);
        self.charge(
            NATIVE_FUNCTION_BASE_COST
                .saturating_add(amount.get())
                .saturating_add(return_cost),
        )''',
        1,
    )
    text = text.replace(
        '''        _ty_args: impl ExactSizeIterator<Item = impl move_vm_types::views::TypeView>,
        _args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge(NATIVE_FUNCTION_PRE_EXEC_COST)''',
        '''        ty_args: impl ExactSizeIterator<Item = impl move_vm_types::views::TypeView>,
        args: impl ExactSizeIterator<Item = impl move_vm_types::views::ValueView>,
    ) -> PartialVMResult<()> {
        self.charge(
            NATIVE_FUNCTION_PRE_EXEC_COST
                .saturating_add(ty_args.len() as u64)
                .saturating_add(args.len() as u64),
        )''',
        1,
    )
    write(meter, text)

    runtime = "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs"
    text = read(runtime)
    publish_old = '''        let (move_changeset, events) = {
            // Separate lock into a variable first to prevent it from being dropped immediately
            let vm_guard = self.read_vm();
            let mut session = self.create_session_with_storage_ext(&vm_guard);

            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);
            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);

            session
                .publish_module(module_bytes.clone(), sender, &mut metered_gas)
                .map_err(|e| anyhow::anyhow!("{:?}", e))?;

            session.finish().0.map_err(|e| anyhow::anyhow!("{:?}", e))?
        };'''
    publish_new = '''        let (move_changeset, events, vm_gas_used) = {
            let vm_guard = self.read_vm();
            let mut session = self.create_session_with_storage_ext(&vm_guard);
            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(100_000);
            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);
            metered_gas
                .charge(module_bytes.len() as u64)
                .map_err(|e| anyhow::anyhow!("{:?}", e))?;
            session
                .publish_module(module_bytes.clone(), sender, &mut metered_gas)
                .map_err(|e| anyhow::anyhow!("{:?}", e))?;
            let used = metered_gas.gas_used();
            let (changes, events) = session.finish().0.map_err(|e| anyhow::anyhow!("{:?}", e))?;
            (changes, events, used)
        };'''
    if publish_old not in text:
        raise RuntimeError("runtime publish metering block not found")
    text = text.replace(publish_old, publish_new, 1)
    text = text.replace(
        '''        if let Some((gas_limit, gas_price)) = gas_info {
            let gas_op = GasOperation::PublishModule {
                module_size: module_bytes.len(),
            };
            let (written, deleted) = self.calculate_storage_impact(&move_changeset, &cs);
            self.apply_gas_info(
                &mut cs,
                Some(sender),
                gas_limit,
                gas_price,
                gas_op,
                written,
                deleted,
            )?;
        }

        Ok(cs)''',
        '''        cs.set_gas_used(vm_gas_used);
        Ok(cs)''',
        1,
    )
    entry_old = '''        let execution_result = if bypass_entry_check {
            let mut unmetered_gas = UnmeteredGasMeter;
            session.execute_function_bypass_visibility(
                module_id,
                ident,
                ty_args_loaded,
                final_args,
                &mut unmetered_gas,
            )
        } else {
            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);
            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);
            session.execute_entry_function(
                module_id,
                ident,
                ty_args_loaded,
                final_args,
                &mut metered_gas,
            )
        };'''
    entry_new = '''        let preprocessing_gas = final_args
            .iter()
            .fold(ty_args_loaded.len() as u64, |total, arg| total.saturating_add(arg.len() as u64));
        let (execution_result, vm_gas_used) = if bypass_entry_check {
            let mut unmetered_gas = UnmeteredGasMeter;
            (
                session.execute_function_bypass_visibility(
                    module_id,
                    ident,
                    ty_args_loaded,
                    final_args,
                    &mut unmetered_gas,
                ),
                0,
            )
        } else {
            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(100_000);
            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);
            metered_gas
                .charge(preprocessing_gas)
                .map_err(|e| anyhow::anyhow!("{:?}", e))?;
            let result = session.execute_entry_function(
                module_id,
                ident,
                ty_args_loaded,
                final_args,
                &mut metered_gas,
            );
            (result, metered_gas.gas_used())
        };'''
    if entry_old not in text:
        raise RuntimeError("runtime entry metering block not found")
    text = text.replace(entry_old, entry_new, 1)
    text, count = re.subn(
        r"\n                if let Some\(\(gas_limit, gas_price\)\) = gas_info \{\n                    let complexity = 1 \+ \(total_merge_reads as u32 / 10\);.*?\n                \}\n",
        "\n                cs.set_gas_used(vm_gas_used.saturating_add(total_merge_reads));\n",
        text,
        count=1,
        flags=re.S,
    )
    if count != 1:
        raise RuntimeError("runtime success gas-info block not found")
    text, count = re.subn(
        r"\n                if let Some\(\(gas_limit, gas_price\)\) = gas_info \{\n                    let penalty_complexity = 5 \+ \(total_merge_reads as u32\);.*?\n                \}\n                Err\(anyhow::anyhow!\(\"exec error: \{:\?\}\", e\)\)",
        '''
                cs.set_gas_used(vm_gas_used.max(1));
                Err(anyhow::anyhow!("exec error: {:?}", e))''',
        text,
        count=1,
        flags=re.S,
    )
    if count != 1:
        raise RuntimeError("runtime failure gas-info block not found")
    write(runtime, text)

    parser = "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/parsers.rs"
    text = read(parser)
    old = '''                        kanari_cs.add_created_object(
                            *addr,
                            format!("{}", struct_tag),
                            bytes.to_vec(),
                            0,
                            uid_opt,
                            id_opt,
                            Some(final_object_id),
                        );'''
    new = '''                        // Ordinary global resources are not objects. Only mirror a
                        // resource write when it is a writeback of an already-known object.
                        // Newly created objects arrive through the SavedObjects native extension.
                        if self.object_storage.get_object(&final_object_id).is_some() {
                            kanari_cs.add_created_object(
                                *addr,
                                format!("{}", struct_tag),
                                bytes.to_vec(),
                                0,
                                uid_opt,
                                id_opt,
                                Some(final_object_id),
                            );
                        } else {
                            debug!("[PARSER] skipping non-object global resource: addr={} type={}", addr, struct_tag);
                        }'''
    if old not in text:
        raise RuntimeError("runtime resource parser object block not found")
    write(parser, text.replace(old, new, 1))
