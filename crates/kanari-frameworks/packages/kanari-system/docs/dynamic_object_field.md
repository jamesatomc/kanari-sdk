
<a name="0x2_dynamic_object_field"></a>

# Module `0x2::dynamic_object_field`



-  [Constants](#@Constants_0)
-  [Function `add`](#0x2_dynamic_object_field_add)
-  [Function `borrow_mut`](#0x2_dynamic_object_field_borrow_mut)
-  [Function `borrow`](#0x2_dynamic_object_field_borrow)
-  [Function `remove`](#0x2_dynamic_object_field_remove)
-  [Function `exists_`](#0x2_dynamic_object_field_exists_)
-  [Function `exists_with_type`](#0x2_dynamic_object_field_exists_with_type)
-  [Function `contains`](#0x2_dynamic_object_field_contains)
-  [Function `attach`](#0x2_dynamic_object_field_attach)
-  [Function `attach_if_missing`](#0x2_dynamic_object_field_attach_if_missing)
-  [Function `detach`](#0x2_dynamic_object_field_detach)
-  [Function `replace`](#0x2_dynamic_object_field_replace)


<pre><code><b>use</b> <a href="object.md#0x2_object">0x2::object</a>;
</code></pre>



<a name="@Constants_0"></a>

## Constants


<a name="0x2_dynamic_object_field_EFieldAlreadyExists"></a>

Error codes


<pre><code><b>const</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_EFieldAlreadyExists">EFieldAlreadyExists</a>: u64 = 1;
</code></pre>



<a name="0x2_dynamic_object_field_EFieldDoesNotExist"></a>



<pre><code><b>const</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_EFieldDoesNotExist">EFieldDoesNotExist</a>: u64 = 2;
</code></pre>



<a name="0x2_dynamic_object_field_ENotObject"></a>



<pre><code><b>const</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_ENotObject">ENotObject</a>: u64 = 3;
</code></pre>



<a name="0x2_dynamic_object_field_add"></a>

## Function `add`



<pre><code><b>public</b> <b>fun</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_add">add</a>&lt;Name: <b>copy</b>, drop, store, Value: store, key&gt;(<a href="object.md#0x2_object">object</a>: &<b>mut</b> <a href="object.md#0x2_object_UID">object::UID</a>, name: Name, value: Value)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>native</b> <b>fun</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_add">add</a>&lt;Name: <b>copy</b> + drop + store, Value: key + store&gt;(
    <a href="object.md#0x2_object">object</a>: &<b>mut</b> UID,
    name: Name,
    value: Value,
);
</code></pre>



</details>

<a name="0x2_dynamic_object_field_exists_with_type"></a>

## Function `exists_with_type`

Returns true only when the object field exists with the requested object type.

<a name="0x2_dynamic_object_field_contains"></a>

## Function `contains`

Convenience alias for checking whether an object field name exists.

<a name="0x2_dynamic_object_field_attach"></a>

## Function `attach`

Attaches a key and store object to the parent object.

<a name="0x2_dynamic_object_field_attach_if_missing"></a>

## Function `attach_if_missing`

Attaches an object only when the name is unused and returns whether it was attached.

<a name="0x2_dynamic_object_field_detach"></a>

## Function `detach`

Detaches an object field and returns the object in an option.

<a name="0x2_dynamic_object_field_replace"></a>

## Function `replace`

Replaces an existing object field and returns the previous object.

<a name="0x2_dynamic_object_field_borrow_mut"></a>

## Function `borrow_mut`



<pre><code><b>public</b> <b>fun</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_borrow_mut">borrow_mut</a>&lt;Name: <b>copy</b>, drop, store, Value: store, key&gt;(<a href="object.md#0x2_object">object</a>: &<b>mut</b> <a href="object.md#0x2_object_UID">object::UID</a>, name: Name): &<b>mut</b> Value
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>native</b> <b>fun</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_borrow_mut">borrow_mut</a>&lt;Name: <b>copy</b> + drop + store, Value: key + store&gt;(
    <a href="object.md#0x2_object">object</a>: &<b>mut</b> UID,
    name: Name,
): &<b>mut</b> Value;
</code></pre>



</details>

<a name="0x2_dynamic_object_field_borrow"></a>

## Function `borrow`



<pre><code><b>public</b> <b>fun</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_borrow">borrow</a>&lt;Name: <b>copy</b>, drop, store, Value: store, key&gt;(<a href="object.md#0x2_object">object</a>: &<a href="object.md#0x2_object_UID">object::UID</a>, name: Name): &Value
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>native</b> <b>fun</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_borrow">borrow</a>&lt;Name: <b>copy</b> + drop + store, Value: key + store&gt;(
    <a href="object.md#0x2_object">object</a>: &UID,
    name: Name,
): &Value;
</code></pre>



</details>

<a name="0x2_dynamic_object_field_remove"></a>

## Function `remove`



<pre><code><b>public</b> <b>fun</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_remove">remove</a>&lt;Name: <b>copy</b>, drop, store, Value: store, key&gt;(<a href="object.md#0x2_object">object</a>: &<b>mut</b> <a href="object.md#0x2_object_UID">object::UID</a>, name: Name): Value
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>native</b> <b>fun</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_remove">remove</a>&lt;Name: <b>copy</b> + drop + store, Value: key + store&gt;(
    <a href="object.md#0x2_object">object</a>: &<b>mut</b> UID,
    name: Name,
): Value;
</code></pre>



</details>

<a name="0x2_dynamic_object_field_exists_"></a>

## Function `exists_`



<pre><code><b>public</b> <b>fun</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_exists_">exists_</a>&lt;Name: <b>copy</b>, drop, store&gt;(<a href="object.md#0x2_object">object</a>: &<a href="object.md#0x2_object_UID">object::UID</a>, name: Name): bool
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>native</b> <b>fun</b> <a href="dynamic_object_field.md#0x2_dynamic_object_field_exists_">exists_</a>&lt;Name: <b>copy</b> + drop + store&gt;(
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
