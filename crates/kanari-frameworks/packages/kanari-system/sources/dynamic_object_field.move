// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::dynamic_object_field {
    use kanari_system::object::UID;
    use std::option::{Self, Option};

    #[allow(unused_const)]
    /// Error codes
    const EFieldAlreadyExists: u64 = 1;
    #[allow(unused_const)]
    const EFieldDoesNotExist: u64 = 2;
    #[allow(unused_const)]
    const ENotObject: u64 = 3;

    public native fun add<Name: copy + drop + store, Value: key + store>(
        object: &mut UID,
        name: Name,
        value: Value,
    );

    public native fun borrow_mut<Name: copy + drop + store, Value: key + store>(
        object: &mut UID,
        name: Name,
    ): &mut Value;

    public native fun borrow<Name: copy + drop + store, Value: key + store>(
        object: &UID,
        name: Name,
    ): &Value;

    public native fun remove<Name: copy + drop + store, Value: key + store>(
        object: &mut UID,
        name: Name,
    ): Value;

    public native fun exists_<Name: copy + drop + store>(
        object: &UID,
        name: Name,
    ): bool;

    /// Returns true when the object field exists with the requested value type.
    /// Matches the Sui dynamic_object_field API.
    public native fun exists_with_type<Name: copy + drop + store, Value: key + store>(
        object: &UID,
        name: Name,
    ): bool;

    /// Returns true if the dynamic object field exists.
    public fun contains<Name: copy + drop + store>(object: &UID, name: Name): bool {
        exists_(object, name)
    }

    /// Attaches an owned object to the parent object under `name`.
    /// Aborts when an object is already attached under the same name.
    public fun attach<Name: copy + drop + store, Value: key + store>(
        object: &mut UID,
        name: Name,
        value: Value,
    ) {
        add(object, name, value);
    }

    /// Adds an object field only when the key is not already present.
    /// Returns false when the field already exists.
    public fun add_if_absent<Name: copy + drop + store, Value: key + store + drop>(
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

    /// Attaches an object only when the name is free.
    /// Returns false when another object already uses the name.
    public fun attach_if_missing<Name: copy + drop + store, Value: key + store + drop>(
        object: &mut UID,
        name: Name,
        value: Value,
    ): bool {
        add_if_absent(object, name, value)
    }

    /// Removes an object field when present and returns its value.
    /// Returns `none` when the field does not exist.
    public fun remove_if_exists<Name: copy + drop + store, Value: key + store>(
        object: &mut UID,
        name: Name,
    ): Option<Value> {
        if (exists_(object, name)) {
            option::some(remove(object, name))
        } else {
            option::none()
        }
    }

    /// Detaches an object when present and returns it to the caller.
    /// Returns `none` when no object is attached under the name.
    public fun detach<Name: copy + drop + store, Value: key + store>(
        object: &mut UID,
        name: Name,
    ): Option<Value> {
        remove_if_exists(object, name)
    }

    /// Replaces an existing object field and returns its previous value.
    public fun replace<Name: copy + drop + store, Value: key + store>(
        object: &mut UID,
        name: Name,
        value: Value,
    ): Value {
        let previous = remove(object, name);
        add(object, name, value);
        previous
    }
}