
<a name="0x2_deny_list"></a>

# Module `0x2::deny_list`



-  [Resource `DenyList`](#0x2_deny_list_DenyList)
-  [Resource `DenyCap`](#0x2_deny_list_DenyCap)
-  [Function `new_denylist`](#0x2_deny_list_new_denylist)
-  [Function `new_denycap`](#0x2_deny_list_new_denycap)
-  [Function `deny_list_add`](#0x2_deny_list_deny_list_add)
-  [Function `deny_list_remove`](#0x2_deny_list_deny_list_remove)


<pre><code><b>use</b> <a href="dependencies/move-stdlib/vector.md#0x1_vector">0x1::vector</a>;
<b>use</b> <a href="object.md#0x2_object">0x2::object</a>;
<b>use</b> <a href="tx_context.md#0x2_tx_context">0x2::tx_context</a>;
</code></pre>



<a name="0x2_deny_list_DenyList"></a>

## Resource `DenyList`

Deny list resource storing addresses


<pre><code><b>struct</b> <a href="deny_list.md#0x2_deny_list_DenyList">DenyList</a> <b>has</b> drop, store, key
</code></pre>



<details>
<summary>Fields</summary>


<dl>
<dt>
<code>addresses: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;<b>address</b>&gt;</code>
</dt>
<dd>

</dd>
</dl>


</details>

<a name="0x2_deny_list_DenyCap"></a>

## Resource `DenyCap`

Capability to mutate a DenyList for a specific coin type


<pre><code><b>struct</b> <a href="deny_list.md#0x2_deny_list_DenyCap">DenyCap</a>&lt;T&gt; <b>has</b> drop, store, key
</code></pre>



<details>
<summary>Fields</summary>


<dl>
<dt>
<code>id: <a href="object.md#0x2_object_UID">object::UID</a></code>
</dt>
<dd>

</dd>
</dl>


</details>

<a name="0x2_deny_list_new_denylist"></a>

## Function `new_denylist`

Create a new empty DenyList


<pre><code><b>public</b> <b>fun</b> <a href="deny_list.md#0x2_deny_list_new_denylist">new_denylist</a>(): <a href="deny_list.md#0x2_deny_list_DenyList">deny_list::DenyList</a>
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="deny_list.md#0x2_deny_list_new_denylist">new_denylist</a>(): <a href="deny_list.md#0x2_deny_list_DenyList">DenyList</a> {
    <a href="deny_list.md#0x2_deny_list_DenyList">DenyList</a> { addresses: <a href="dependencies/move-stdlib/vector.md#0x1_vector_empty">vector::empty</a>&lt;<b>address</b>&gt;() }
}
</code></pre>



</details>

<a name="0x2_deny_list_new_denycap"></a>

## Function `new_denycap`

Create a DenyCap while constructing a regulated currency.  Keeping this
friend-only prevents an arbitrary transaction from forging a capability
for another asset type and changing its deny list.


<pre><code><b>public</b>(<b>friend</b>) <b>fun</b> <a href="deny_list.md#0x2_deny_list_new_denycap">new_denycap</a>&lt;T&gt;(ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>): <a href="deny_list.md#0x2_deny_list_DenyCap">deny_list::DenyCap</a>&lt;T&gt;
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b>(<b>friend</b>) <b>fun</b> <a href="deny_list.md#0x2_deny_list_new_denycap">new_denycap</a>&lt;T&gt;(ctx: &<b>mut</b> TxContext): <a href="deny_list.md#0x2_deny_list_DenyCap">DenyCap</a>&lt;T&gt; {
    <a href="deny_list.md#0x2_deny_list_DenyCap">DenyCap</a>&lt;T&gt; { id: <a href="object.md#0x2_object_new">object::new</a>(ctx) }
}
</code></pre>



</details>

<a name="0x2_deny_list_deny_list_add"></a>

## Function `deny_list_add`

Add an address to the deny list


<pre><code><b>public</b> <b>fun</b> <a href="deny_list.md#0x2_deny_list_deny_list_add">deny_list_add</a>&lt;T&gt;(d: &<b>mut</b> <a href="deny_list.md#0x2_deny_list_DenyList">deny_list::DenyList</a>, _cap: &<a href="deny_list.md#0x2_deny_list_DenyCap">deny_list::DenyCap</a>&lt;T&gt;, addr: <b>address</b>, _ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="deny_list.md#0x2_deny_list_deny_list_add">deny_list_add</a>&lt;T&gt;(d: &<b>mut</b> <a href="deny_list.md#0x2_deny_list_DenyList">DenyList</a>, _cap: &<a href="deny_list.md#0x2_deny_list_DenyCap">DenyCap</a>&lt;T&gt;, addr: <b>address</b>, _ctx: &<b>mut</b> TxContext) {
    // Check <b>if</b> <b>address</b> already exists in the deny list
    <b>let</b> len = <a href="dependencies/move-stdlib/vector.md#0x1_vector_length">vector::length</a>(&d.addresses);
    <b>let</b>  i = 0;
    <b>while</b> (i &lt; len) {
        <b>let</b> existing_addr = *<a href="dependencies/move-stdlib/vector.md#0x1_vector_borrow">vector::borrow</a>(&d.addresses, i);
        <b>if</b> (existing_addr == addr) {
            // Address already exists, no need <b>to</b> add again
            <b>return</b>
        };
        i = i + 1;
    };

    // Address not found, add it <b>to</b> the deny list
    <a href="dependencies/move-stdlib/vector.md#0x1_vector_push_back">vector::push_back</a>(&<b>mut</b> d.addresses, addr);
}
</code></pre>



</details>

<a name="0x2_deny_list_deny_list_remove"></a>

## Function `deny_list_remove`

Remove an address from the deny list


<pre><code><b>public</b> <b>fun</b> <a href="deny_list.md#0x2_deny_list_deny_list_remove">deny_list_remove</a>&lt;T&gt;(d: &<b>mut</b> <a href="deny_list.md#0x2_deny_list_DenyList">deny_list::DenyList</a>, _cap: &<a href="deny_list.md#0x2_deny_list_DenyCap">deny_list::DenyCap</a>&lt;T&gt;, addr: <b>address</b>, _ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="deny_list.md#0x2_deny_list_deny_list_remove">deny_list_remove</a>&lt;T&gt;(d: &<b>mut</b> <a href="deny_list.md#0x2_deny_list_DenyList">DenyList</a>, _cap: &<a href="deny_list.md#0x2_deny_list_DenyCap">DenyCap</a>&lt;T&gt;, addr: <b>address</b>, _ctx: &<b>mut</b> TxContext) {
    <b>let</b> len = <a href="dependencies/move-stdlib/vector.md#0x1_vector_length">vector::length</a>(&d.addresses);
    <b>let</b> i = 0;
    <b>while</b> (i &lt; len) {
        <b>let</b> existing_addr = *<a href="dependencies/move-stdlib/vector.md#0x1_vector_borrow">vector::borrow</a>(&d.addresses, i);
        <b>if</b> (existing_addr == addr) {
            // Found the <b>address</b>, remove it from the <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>
            <a href="dependencies/move-stdlib/vector.md#0x1_vector_remove">vector::remove</a>(&<b>mut</b> d.addresses, i);
            <b>return</b>
        };
        i = i + 1;
    };
    // If <b>address</b> not found, do nothing (no-op)
}
</code></pre>



</details>


[//]: # ("File containing references which can be used from documentation")

[Move Language]: https://github.com/move-language/move
[Kanari]: https://github.com/jamesatomc/kanari-cp
[Move Book]: https://move-language.github.io/move/
[Transfer Module]: transfer.md
