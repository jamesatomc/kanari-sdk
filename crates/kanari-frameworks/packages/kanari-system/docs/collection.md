
<a name="0x2_collection"></a>

# Module `0x2::collection`



-  [Resource `Collection`](#0x2_collection_Collection)
-  [Resource `NftCap`](#0x2_collection_NftCap)
-  [Struct `CollectionCreated`](#0x2_collection_CollectionCreated)
-  [Constants](#@Constants_0)
-  [Function `create_collection`](#0x2_collection_create_collection)
-  [Function `collection_id`](#0x2_collection_collection_id)
-  [Function `cap_collection_id`](#0x2_collection_cap_collection_id)
-  [Function `collection_creator`](#0x2_collection_collection_creator)
-  [Function `max_supply`](#0x2_collection_max_supply)
-  [Function `update_metadata`](#0x2_collection_update_metadata)
-  [Function `remaining`](#0x2_collection_remaining)
-  [Function `issued`](#0x2_collection_issued)
-  [Function `consume_for_mint`](#0x2_collection_consume_for_mint)
-  [Function `return_from_burn`](#0x2_collection_return_from_burn)
-  [Function `transfer_collection`](#0x2_collection_transfer_collection)
-  [Function `transfer_cap`](#0x2_collection_transfer_cap)


<pre><code><b>use</b> <a href="dependencies/move-stdlib/string.md#0x1_string">0x1::string</a>;
<b>use</b> <a href="event.md#0x2_event">0x2::event</a>;
<b>use</b> <a href="object.md#0x2_object">0x2::object</a>;
<b>use</b> <a href="transfer.md#0x2_transfer">0x2::transfer</a>;
<b>use</b> <a href="tx_context.md#0x2_tx_context">0x2::tx_context</a>;
<b>use</b> <a href="url.md#0x2_url">0x2::url</a>;
</code></pre>



<a name="0x2_collection_Collection"></a>

## Resource `Collection`

A reusable Collection resource for NFTs and similar objects.


<pre><code><b>struct</b> <a href="collection.md#0x2_collection_Collection">Collection</a> <b>has</b> store, key
</code></pre>



<details>
<summary>Fields</summary>


<dl>
<dt>
<code>id: <a href="object.md#0x2_object_UID">object::UID</a></code>
</dt>
<dd>

</dd>
<dt>
<code>name: <a href="dependencies/move-stdlib/string.md#0x1_string_String">string::String</a></code>
</dt>
<dd>

</dd>
<dt>
<code>description: <a href="dependencies/move-stdlib/string.md#0x1_string_String">string::String</a></code>
</dt>
<dd>

</dd>
<dt>
<code>banner_url: <a href="url.md#0x2_url_Url">url::Url</a></code>
</dt>
<dd>

</dd>
<dt>
<code>website_url: <a href="url.md#0x2_url_Url">url::Url</a></code>
</dt>
<dd>

</dd>
<dt>
<code>creator: <b>address</b></code>
</dt>
<dd>

</dd>
<dt>
<code>max_supply: u64</code>
</dt>
<dd>

</dd>
</dl>


</details>

<a name="0x2_collection_NftCap"></a>

## Resource `NftCap`

A capability resource that governs minting within a Collection.


<pre><code><b>struct</b> <a href="collection.md#0x2_collection_NftCap">NftCap</a> <b>has</b> drop, store, key
</code></pre>



<details>
<summary>Fields</summary>


<dl>
<dt>
<code>id: <a href="object.md#0x2_object_UID">object::UID</a></code>
</dt>
<dd>

</dd>
<dt>
<code>remaining: u64</code>
</dt>
<dd>

</dd>
<dt>
<code>issued_counter: u64</code>
</dt>
<dd>

</dd>
<dt>
<code>collection_id: <b>address</b></code>
</dt>
<dd>

</dd>
</dl>


</details>

<a name="0x2_collection_CollectionCreated"></a>

## Struct `CollectionCreated`

Event emitted when a collection is created (for off-chain indexing)


<pre><code><b>struct</b> <a href="collection.md#0x2_collection_CollectionCreated">CollectionCreated</a> <b>has</b> <b>copy</b>, drop
</code></pre>



<details>
<summary>Fields</summary>


<dl>
<dt>
<code>collection_id: <b>address</b></code>
</dt>
<dd>

</dd>
<dt>
<code>creator: <b>address</b></code>
</dt>
<dd>

</dd>
<dt>
<code>max_supply: u64</code>
</dt>
<dd>

</dd>
</dl>


</details>

<a name="@Constants_0"></a>

## Constants


<a name="0x2_collection_E_NOT_COLLECTION_CREATOR"></a>



<pre><code><b>const</b> <a href="collection.md#0x2_collection_E_NOT_COLLECTION_CREATOR">E_NOT_COLLECTION_CREATOR</a>: u64 = 2;
</code></pre>



<a name="0x2_collection_E_NO_SUPPLY"></a>



<pre><code><b>const</b> <a href="collection.md#0x2_collection_E_NO_SUPPLY">E_NO_SUPPLY</a>: u64 = 1;
</code></pre>



<a name="0x2_collection_create_collection"></a>

## Function `create_collection`

Create a collection and its corresponding <code><a href="collection.md#0x2_collection_NftCap">NftCap</a></code>.
Returns <code>(<a href="collection.md#0x2_collection_Collection">Collection</a>, <a href="collection.md#0x2_collection_NftCap">NftCap</a>)</code> so callers can persist one or both.


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_create_collection">create_collection</a>(ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>, name: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, description: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, banner_url: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, website_url: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, max_supply: u64): (<a href="collection.md#0x2_collection_Collection">collection::Collection</a>, <a href="collection.md#0x2_collection_NftCap">collection::NftCap</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_create_collection">create_collection</a>(
    ctx: &<b>mut</b> TxContext,
    name: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    description: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    banner_url: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    website_url: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    max_supply: u64,
): (<a href="collection.md#0x2_collection_Collection">Collection</a>, <a href="collection.md#0x2_collection_NftCap">NftCap</a>) {
    <b>let</b> id = <a href="object.md#0x2_object_new">object::new</a>(ctx);
    <b>let</b> sender = <a href="tx_context.md#0x2_tx_context_sender">tx_context::sender</a>(ctx);

    <b>let</b> collection_addr = <a href="object.md#0x2_object_uid_address">object::uid_address</a>(&id);

    <b>let</b> coll = <a href="collection.md#0x2_collection_Collection">Collection</a> {
        id,
        name: utf8(name),
        description: utf8(description),
        banner_url: kanari_system::url::new_unsafe_from_bytes(banner_url),
        website_url: kanari_system::url::new_unsafe_from_bytes(website_url),
        creator: sender,
        max_supply,
    };

    <b>let</b> cap = <a href="collection.md#0x2_collection_NftCap">NftCap</a> {
        id: <a href="object.md#0x2_object_new">object::new</a>(ctx),
        remaining: max_supply,
        issued_counter: 0,
        collection_id: collection_addr, // ใช้ <b>address</b> ที่ดึงมา
    };

    <a href="event.md#0x2_event_emit">event::emit</a>(<a href="collection.md#0x2_collection_CollectionCreated">CollectionCreated</a> {
        collection_id: collection_addr,
        creator: sender,
        max_supply
    });

    (coll, cap)
}
</code></pre>



</details>

<a name="0x2_collection_collection_id"></a>

## Function `collection_id`

Returns the address (UID) of a <code><a href="collection.md#0x2_collection_Collection">Collection</a></code>.


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_collection_id">collection_id</a>(_c: &<a href="collection.md#0x2_collection_Collection">collection::Collection</a>): <b>address</b>
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_collection_id">collection_id</a>(_c: &<a href="collection.md#0x2_collection_Collection">Collection</a>): <b>address</b> {
    // Consumers can call `<a href="object.md#0x2_object_uid_address">object::uid_address</a>(&c.id)` directly; keep API minimal.
    <a href="object.md#0x2_object_uid_address">object::uid_address</a>(&_c.id)
}
</code></pre>



</details>

<a name="0x2_collection_cap_collection_id"></a>

## Function `cap_collection_id`

Returns the collection id stored in an <code><a href="collection.md#0x2_collection_NftCap">NftCap</a></code>.


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_cap_collection_id">cap_collection_id</a>(cap: &<a href="collection.md#0x2_collection_NftCap">collection::NftCap</a>): <b>address</b>
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_cap_collection_id">cap_collection_id</a>(cap: &<a href="collection.md#0x2_collection_NftCap">NftCap</a>): <b>address</b> {
    cap.collection_id
}
</code></pre>



</details>

<a name="0x2_collection_collection_creator"></a>

## Function `collection_creator`



<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_collection_creator">collection_creator</a>(c: &<a href="collection.md#0x2_collection_Collection">collection::Collection</a>): <b>address</b>
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_collection_creator">collection_creator</a>(c: &<a href="collection.md#0x2_collection_Collection">Collection</a>): <b>address</b> {
    c.creator
}
</code></pre>



</details>

<a name="0x2_collection_max_supply"></a>

## Function `max_supply`



<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_max_supply">max_supply</a>(c: &<a href="collection.md#0x2_collection_Collection">collection::Collection</a>): u64
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_max_supply">max_supply</a>(c: &<a href="collection.md#0x2_collection_Collection">Collection</a>): u64 {
    c.max_supply
}
</code></pre>



</details>

<a name="0x2_collection_update_metadata"></a>

## Function `update_metadata`

Updates collection display metadata. Only the original collection creator
may perform this mutation; supply and ownership are intentionally unchanged.


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_update_metadata">update_metadata</a>(c: &<b>mut</b> <a href="collection.md#0x2_collection_Collection">collection::Collection</a>, banner_url: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, website_url: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, ctx: &<a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_update_metadata">update_metadata</a>(
    c: &<b>mut</b> <a href="collection.md#0x2_collection_Collection">Collection</a>,
    banner_url: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    website_url: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    ctx: &TxContext,
) {
    <b>assert</b>!(<a href="tx_context.md#0x2_tx_context_sender">tx_context::sender</a>(ctx) == c.creator, <a href="collection.md#0x2_collection_E_NOT_COLLECTION_CREATOR">E_NOT_COLLECTION_CREATOR</a>);
    c.banner_url = kanari_system::url::new_unsafe_from_bytes(banner_url);
    c.website_url = kanari_system::url::new_unsafe_from_bytes(website_url);
    <a href="object.md#0x2_object_save_object">object::save_object</a>(c);
}
</code></pre>



</details>

<a name="0x2_collection_remaining"></a>

## Function `remaining`



<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_remaining">remaining</a>(cap: &<a href="collection.md#0x2_collection_NftCap">collection::NftCap</a>): u64
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_remaining">remaining</a>(cap: &<a href="collection.md#0x2_collection_NftCap">NftCap</a>): u64 {
    cap.remaining
}
</code></pre>



</details>

<a name="0x2_collection_issued"></a>

## Function `issued`



<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_issued">issued</a>(cap: &<a href="collection.md#0x2_collection_NftCap">collection::NftCap</a>): u64
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_issued">issued</a>(cap: &<a href="collection.md#0x2_collection_NftCap">NftCap</a>): u64 {
    cap.issued_counter
}
</code></pre>



</details>

<a name="0x2_collection_consume_for_mint"></a>

## Function `consume_for_mint`

Consume one supply unit from the cap for minting.


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_consume_for_mint">consume_for_mint</a>(cap: &<b>mut</b> <a href="collection.md#0x2_collection_NftCap">collection::NftCap</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_consume_for_mint">consume_for_mint</a>(cap: &<b>mut</b> <a href="collection.md#0x2_collection_NftCap">NftCap</a>) {
    <b>assert</b>!(cap.remaining &gt; 0, <a href="collection.md#0x2_collection_E_NO_SUPPLY">E_NO_SUPPLY</a>);
    cap.issued_counter = cap.issued_counter + 1;
    cap.remaining = cap.remaining - 1;
    <a href="object.md#0x2_object_save_object">object::save_object</a>(cap);
}
</code></pre>



</details>

<a name="0x2_collection_return_from_burn"></a>

## Function `return_from_burn`

Return one supply unit to cap (used on burn).


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_return_from_burn">return_from_burn</a>(cap: &<b>mut</b> <a href="collection.md#0x2_collection_NftCap">collection::NftCap</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_return_from_burn">return_from_burn</a>(cap: &<b>mut</b> <a href="collection.md#0x2_collection_NftCap">NftCap</a>) {
    cap.remaining = cap.remaining + 1;
    // Note: issued_counter is intentionally not decremented; it records how many
    // items have been minted historically.
    <a href="object.md#0x2_object_save_object">object::save_object</a>(cap);
}
</code></pre>



</details>

<a name="0x2_collection_transfer_collection"></a>

## Function `transfer_collection`

Get the creator of a collection.
Transfer helpers using <code><a href="transfer.md#0x2_transfer_public_transfer">transfer::public_transfer</a></code>.


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_transfer_collection">transfer_collection</a>(c: <a href="collection.md#0x2_collection_Collection">collection::Collection</a>, recipient: <b>address</b>, _ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_transfer_collection">transfer_collection</a>(c: <a href="collection.md#0x2_collection_Collection">Collection</a>, recipient: <b>address</b>, _ctx: &<b>mut</b> TxContext) {
    <a href="transfer.md#0x2_transfer_public_transfer">transfer::public_transfer</a>(c, recipient)
}
</code></pre>



</details>

<a name="0x2_collection_transfer_cap"></a>

## Function `transfer_cap`



<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_transfer_cap">transfer_cap</a>(cap: <a href="collection.md#0x2_collection_NftCap">collection::NftCap</a>, recipient: <b>address</b>, _ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="collection.md#0x2_collection_transfer_cap">transfer_cap</a>(cap: <a href="collection.md#0x2_collection_NftCap">NftCap</a>, recipient: <b>address</b>, _ctx: &<b>mut</b> TxContext) {
    <a href="transfer.md#0x2_transfer_public_transfer">transfer::public_transfer</a>(cap, recipient)
}
</code></pre>



</details>


[//]: # ("File containing references which can be used from documentation")

[Move Language]: https://github.com/move-language/move
[Kanari]: https://github.com/jamesatomc/kanari-cp
[Move Book]: https://move-language.github.io/move/
[Transfer Module]: transfer.md
