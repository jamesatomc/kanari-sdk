// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use better_any::TidAble;
use move_vm_runtime::native_functions::NativeContext;

/// Run `f` with a mutable native-context extension of type `E`.
///
/// Some hosts (e.g. Move unit-test runner) might not pre-register extensions.
/// `NativeContextExtensions::get_mut::<E>()` panics when the extension is absent.
/// To keep natives robust, this helper will attempt to insert `E::default()` and retry.
///
/// Returns `None` if the extension cannot be obtained (or if `f` panics).
pub(crate) fn with_ext_mut_or_default<'a, 'b, E, R>(
    context: &mut NativeContext<'a, 'b>,
    f: impl FnOnce(&mut E) -> R,
) -> Option<R>
where
    E: TidAble<'b> + Default,
{
    let exts = context.extensions_mut();

    let has_ext = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = exts.get_mut::<E>();
    }))
    .is_ok();

    if !has_ext {
        exts.add(E::default());
    }

    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let ext = exts.get_mut::<E>();
        f(ext)
    }))
    .ok()
}
