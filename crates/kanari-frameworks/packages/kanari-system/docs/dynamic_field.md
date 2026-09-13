
<a name="0x2_dynamic_field"></a>

# Module `0x2::dynamic_field`



-  [Constants](#@Constants_0)
-  [Function `add`](#0x2_dynamic_field_add)
-  [Function `borrow_mut`](#0x2_dynamic_field_borrow_mut)
-  [Function `borrow`](#0x2_dynamic_field_borrow)
-  [Function `remove`](#0x2_dynamic_field_remove)
-  [Function `exists_`](#0x2_dynamic_field_exists_)
-  [Function `exists_with_type`](#0x2_dynamic_field_exists_with_type)
-  [Function `contains`](#0x2_dynamic_field_contains)
-  [Function `add_if_absent`](#0x2_dynamic_field_add_if_absent)
-  [Function `remove_if_exists`](#0x2_dynamic_field_remove_if_exists)
-  [Function `replace`](#0x2_dynamic_field_replace)


<pre><code><b>use</b> <a href="object.md#0x2_object">0x2::object</a>;
</code></pre>



<a name="@Constants_0"></a>

## Constants


<a name="0x2_dynamic_field_EFieldAlreadyExists"></a>

Error codes


<pre><code><b>const</b> <a href="dynamic_field.md#0x2_dynamic_field_EFieldAlreadyExists">EFieldAlreadyExists</a>: u64 = 1;
</code></pre>



<a name="0x2_dynamic_field_EFieldDoesNotExist"></a>



<pre><code><b>const</b> <a href="dynamic_field.md#0x2_dynamic_field_EFieldDoesNotExist">EFieldDoesNotExist</a>: u64 = 2;
</code></pre>



<a name="0x2_dynamic_field_add"></a>

## Function `add`

Adds a dynamic field to the object <code><a href="object.md#0x2_object">object</a></code> with key <code>name</code> and value <code>value</code>.
Aborts with <code><a href="dynamic_field.md#0x2_dynamic_field_EFieldAlreadyExists">EFieldAlreadyExists</a></code> if the object already has a field with that key.


<pre><code><b>public</b> <b>fun</b> <a href="dynamic_field.md#0x2_dynamic_field_add">add</a>&lt;Name: <b>copy</b>, drop, store, Value: store&gt;(<a href="object.md#0x2_object">object</a>: &<b>mut</b> <a href="object.md#0x2_object_UID">object::UID</a>, name: Name, value: Value)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>native</b> <b>fun</b> <a href="dynamic_field.md#0x2_dynamic_field_add">add</a>&lt;Name: <b>copy</b> + drop + store, Value: store&gt;(
    <a href="object.md#0x2_object">object</a>: &<b>mut</b> UID,
    name: Name,
    value: Value,
);
</code></pre>



</details>

<a name="0x2_dynamic_field_exists_with_type"></a>

## Function `exists_with_type`

Returns true only when the field exists with the requested value type.

<pre><code><b>public</b> <b>native</b> <b>fun</b> exists_with_type&lt;Name: <b>copy</b> + drop + store, Value: store&gt;(
    object: &<b>UID</b>, name: Name
): bool;
</code></pre>

<a name="0x2_dynamic_field_contains"></a>

## Function `contains`

Convenience alias for checking whether a field name exists.

<a name="0x2_dynamic_field_add_if_absent"></a>

## Function `add_if_absent`

Adds a field only when the name is unused and returns whether it was added.

<a name="0x2_dynamic_field_remove_if_exists"></a>

## Function `remove_if_exists`

Removes an existing field and returns its value in an option.

<a name="0x2_dynamic_field_replace"></a>

## Function `replace`

Replaces an existing field and returns the previous value.

<a name="0x2_dynamic_field_borrow_mut"></a>

## Function `borrow_mut`

Mutably borrows the dynamic field associated with <code>name</code> on <code><a href="object.md#0x2_object">object</a></code>.
Aborts with <code><a href="dynamic_field.md#0x2_dynamic_field_EFieldDoesNotExist">EFieldDoesNotExist</a></code> if the field does not exist.


<pre><code><b>public</b> <b>fun</b> <a href="dynamic_field.md#0x2_dynamic_field_borrow_mut">borrow_mut</a>&lt;Name: <b>copy</b>, drop, store, Value: store&gt;(<a href="object.md#0x2_object">object</a>: &<b>mut</b> <a href="object.md#0x2_object_UID">object::UID</a>, name: Name): &<b>mut</b> Value
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>native</b> <b>fun</b> <a href="dynamic_field.md#0x2_dynamic_field_borrow_mut">borrow_mut</a>&lt;Name: <b>copy</b> + drop + store, Value: store&gt;(
    <a href="object.md#0x2_object">object</a>: &<b>mut</b> UID,
    name: Name,
): &<b>mut</b> Value;
</code></pre>



</details>

<a name="0x2_dynamic_field_borrow"></a>

## Function `borrow`

Immutably borrows the dynamic field associated with <code>name</code> on <code><a href="object.md#0x2_object">object</a></code>.
Aborts with <code><a href="dynamic_field.md#0x2_dynamic_field_EFieldDoesNotExist">EFieldDoesNotExist</a></code> if the field does not exist.


<pre><code><b>public</b> <b>fun</b> <a href="dynamic_field.md#0x2_dynamic_field_borrow">borrow</a>&lt;Name: <b>copy</b>, drop, store, Value: store&gt;(<a href="object.md#0x2_object">object</a>: &<a href="object.md#0x2_object_UID">object::UID</a>, name: Name): &Value
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>native</b> <b>fun</b> <a href="dynamic_field.md#0x2_dynamic_field_borrow">borrow</a>&lt;Name: <b>copy</b> + drop + store, Value: store&gt;(
    <a href="object.md#0x2_object">object</a>: &UID,
    name: Name,
): &Value;
</code></pre>



</details>

<a name="0x2_dynamic_field_remove"></a>

## Function `remove`

Removes the dynamic field associated with <code>name</code> on <code><a href="object.md#0x2_object">object</a></code> and returns the value.
Aborts with <code><a href="dynamic_field.md#0x2_dynamic_field_EFieldDoesNotExist">EFieldDoesNotExist</a></code> if the field does not exist.


<pre><code><b>public</b> <b>fun</b> <a href="dynamic_field.md#0x2_dynamic_field_remove">remove</a>&lt;Name: <b>copy</b>, drop, store, Value: store&gt;(<a href="object.md#0x2_object">object</a>: &<b>mut</b> <a href="object.md#0x2_object_UID">object::UID</a>, name: Name): Value
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>native</b> <b>fun</b> <a href="dynamic_field.md#0x2_dynamic_field_remove">remove</a>&lt;Name: <b>copy</b> + drop + store, Value: store&gt;(
    <a href="object.md#0x2_object">object</a>: &<b>mut</b> UID,
    name: Name,
): Value;
</code></pre>



</details>

<a name="0x2_dynamic_field_exists_"></a>

## Function `exists_`

Returns true if <code><a href="object.md#0x2_object">object</a></code> has a dynamic field with key <code>name</code>.


<pre><code><b>public</b> <b>fun</b> <a href="dynamic_field.md#0x2_dynamic_field_exists_">exists_</a>&lt;Name: <b>copy</b>, drop, store&gt;(<a href="object.md#0x2_object">object</a>: &<a href="object.md#0x2_object_UID">object::UID</a>, name: Name): bool
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>native</b> <b>fun</b> <a href="dynamic_field.md#0x2_dynamic_field_exists_">exists_</a>&lt;Name: <b>copy</b> + drop + store&gt;(
    <a href="object.md#0x2_object">object</a>: &UID,
    name: Name,
): bool;
</code></pre>



</details>


[//]: # ("File containing references which can be used from documentation")

[Move Language]: https://github.com/move-language/move
[Kanari]: https://github.com/jamesatomc/kanari-cp
[Move Book]: https://move-language.github.io/move/
[Transfer Module]: transfer.md
