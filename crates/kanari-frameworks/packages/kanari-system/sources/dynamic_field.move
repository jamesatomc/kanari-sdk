// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::dynamic_field {
    use kanari_system::object::UID;
    use std::option::{Self, Option};

    #[allow(unused_const)]
    /// Error codes
    const EFieldAlreadyExists: u64 = 1;
    #[allow(unused_const)]
    const EFieldDoesNotExist: u64 = 2;

    /// Adds a dynamic field to the object `object` with key `name` and value `value`.
    /// Aborts with `EFieldAlreadyExists` if the object already has a field with that key.
    public native fun add<Name: copy + drop + store, Value: store>(
        object: &mut UID,
        name: Name,
        value: Value,
    );

    /// Mutably borrows the dynamic field associated with `name` on `object`.
    /// Aborts with `EFieldDoesNotExist` if the field does not exist.
    public native fun borrow_mut<Name: copy + drop + store, Value: store>(
        object: &mut UID,
        name: Name,
    ): &mut Value;

    /// Immutably borrows the dynamic field associated with `name` on `object`.
    /// Aborts with `EFieldDoesNotExist` if the field does not exist.
    public native fun borrow<Name: copy + drop + store, Value: store>(
        object: &UID,
        name: Name,
    ): &Value;

    /// Removes the dynamic field associated with `name` on `object` and returns the value.
    /// Aborts with `EFieldDoesNotExist` if the field does not exist.
    public native fun remove<Name: copy + drop + store, Value: store>(
        object: &mut UID,
        name: Name,
    ): Value;

    /// Returns true if `object` has a dynamic field with key `name`.
    public native fun exists_<Name: copy + drop + store>(
        object: &UID,
        name: Name,
    ): bool;

    /// Returns true when the field exists with the requested value type.
    /// Matches the Sui dynamic_field API.
    public native fun exists_with_type<Name: copy + drop + store, Value: store>(
        object: &UID,
        name: Name,
    ): bool;

    /// Returns true if the dynamic field exists.
    public fun contains<Name: copy + drop + store>(object: &UID, name: Name): bool {
        exists_(object, name)
    }

    /// Adds a field only when the key is not already present.
    /// Returns false when the field already exists.
    public fun add_if_absent<Name: copy + drop + store, Value: drop + store>(
        object: &mut UID,
        name: Name,
        value: Value,
    ): bool {
        if (exists_(object, name)) {
            false
        } else {
            add(object, name, value);
            true
        }
    }

    /// Removes a field when present and returns its value.
    /// Returns `none` when the field does not exist.
    public fun remove_if_exists<Name: copy + drop + store, Value: store>(
        object: &mut UID,
        name: Name,
    ): Option<Value> {
        if (exists_(object, name)) {
            option::some(remove(object, name))
        } else {
            option::none()
        }
    }

    /// Replaces an existing field and returns its previous value.
    public fun replace<Name: copy + drop + store, Value: store>(
        object: &mut UID,
        name: Name,
        value: Value,
    ): Value {
        let previous = remove(object, name);
        add(object, name, value);
        previous
    }
}