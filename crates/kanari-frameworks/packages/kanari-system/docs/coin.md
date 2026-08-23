
<a name="0x2_coin"></a>

# Module `0x2::coin`



-  [Resource `Coin`](#0x2_coin_Coin)
-  [Resource `TreasuryCap`](#0x2_coin_TreasuryCap)
-  [Resource `CoinMetadata`](#0x2_coin_CoinMetadata)
-  [Constants](#@Constants_0)
-  [Function `create_currency`](#0x2_coin_create_currency)
-  [Function `create_regulated_currency`](#0x2_coin_create_regulated_currency)
-  [Function `mint`](#0x2_coin_mint)
-  [Function `mint_and_transfer`](#0x2_coin_mint_and_transfer)
-  [Function `burn`](#0x2_coin_burn)
-  [Function `into_balance`](#0x2_coin_into_balance)
-  [Function `from_balance`](#0x2_coin_from_balance)
-  [Function `total_supply`](#0x2_coin_total_supply)
-  [Function `value`](#0x2_coin_value)
-  [Function `zero`](#0x2_coin_zero)
-  [Function `split`](#0x2_coin_split)
-  [Function `join`](#0x2_coin_join)
-  [Function `join_entry`](#0x2_coin_join_entry)
-  [Function `destroy_zero`](#0x2_coin_destroy_zero)
-  [Function `divide_into_n`](#0x2_coin_divide_into_n)
-  [Function `update_icon_url`](#0x2_coin_update_icon_url)
-  [Function `update_name`](#0x2_coin_update_name)
-  [Function `update_symbol`](#0x2_coin_update_symbol)
-  [Function `update_description`](#0x2_coin_update_description)


<pre><code><b>use</b> <a href="dependencies/move-stdlib/ascii.md#0x1_ascii">0x1::ascii</a>;
<b>use</b> <a href="dependencies/move-stdlib/option.md#0x1_option">0x1::option</a>;
<b>use</b> <a href="dependencies/move-stdlib/string.md#0x1_string">0x1::string</a>;
<b>use</b> <a href="balance.md#0x2_balance">0x2::balance</a>;
<b>use</b> <a href="deny_list.md#0x2_deny_list">0x2::deny_list</a>;
<b>use</b> <a href="object.md#0x2_object">0x2::object</a>;
<b>use</b> <a href="transfer.md#0x2_transfer">0x2::transfer</a>;
<b>use</b> <a href="tx_context.md#0x2_tx_context">0x2::tx_context</a>;
<b>use</b> <a href="url.md#0x2_url">0x2::url</a>;
</code></pre>



<a name="0x2_coin_Coin"></a>

## Resource `Coin`

Coin resource wrapper with balance


<pre><code><b>struct</b> <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt; <b>has</b> drop, store, key
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
<code><a href="balance.md#0x2_balance">balance</a>: <a href="balance.md#0x2_balance_Balance">balance::Balance</a>&lt;T&gt;</code>
</dt>
<dd>

</dd>
</dl>


</details>

<a name="0x2_coin_TreasuryCap"></a>

## Resource `TreasuryCap`

Capability allowing the bearer to mint and burn coins


<pre><code><b>struct</b> <a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt; <b>has</b> drop, store, key
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
<code>total_supply: u64</code>
</dt>
<dd>

</dd>
</dl>


</details>

<a name="0x2_coin_CoinMetadata"></a>

## Resource `CoinMetadata`

Metadata resource for a currency (stored as an object with UID)


<pre><code><b>struct</b> <a href="coin.md#0x2_coin_CoinMetadata">CoinMetadata</a>&lt;T&gt; <b>has</b> drop, store, key
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
<code>decimals: u8</code>
</dt>
<dd>

</dd>
<dt>
<code>name: <a href="dependencies/move-stdlib/string.md#0x1_string_String">string::String</a></code>
</dt>
<dd>

</dd>
<dt>
<code>symbol: <a href="dependencies/move-stdlib/ascii.md#0x1_ascii_String">ascii::String</a></code>
</dt>
<dd>

</dd>
<dt>
<code>description: <a href="dependencies/move-stdlib/string.md#0x1_string_String">string::String</a></code>
</dt>
<dd>

</dd>
<dt>
<code>icon_url: <a href="dependencies/move-stdlib/option.md#0x1_option_Option">option::Option</a>&lt;<a href="url.md#0x2_url_Url">url::Url</a>&gt;</code>
</dt>
<dd>

</dd>
</dl>


</details>

<a name="@Constants_0"></a>

## Constants


<a name="0x2_coin_EINVALID_DECIMALS"></a>

Invalid decimal count.


<pre><code><b>const</b> <a href="coin.md#0x2_coin_EINVALID_DECIMALS">EINVALID_DECIMALS</a>: u64 = 6;
</code></pre>



<a name="0x2_coin_EInvalidArg"></a>

Invalid arguments are passed to a function.


<pre><code><b>const</b> <a href="coin.md#0x2_coin_EInvalidArg">EInvalidArg</a>: u64 = 1;
</code></pre>



<a name="0x2_coin_ENotEnough"></a>

Trying to split a coin more times than its balance allows.


<pre><code><b>const</b> <a href="coin.md#0x2_coin_ENotEnough">ENotEnough</a>: u64 = 2;
</code></pre>



<a name="0x2_coin_EOVERFLOW"></a>

Arithmetic overflow.


<pre><code><b>const</b> <a href="coin.md#0x2_coin_EOVERFLOW">EOVERFLOW</a>: u64 = 4;
</code></pre>



<a name="0x2_coin_EUNDERFLOW"></a>

Arithmetic underflow.


<pre><code><b>const</b> <a href="coin.md#0x2_coin_EUNDERFLOW">EUNDERFLOW</a>: u64 = 5;
</code></pre>



<a name="0x2_coin_EZERO_AMOUNT"></a>

Amount must be greater than zero.


<pre><code><b>const</b> <a href="coin.md#0x2_coin_EZERO_AMOUNT">EZERO_AMOUNT</a>: u64 = 3;
</code></pre>



<a name="0x2_coin_create_currency"></a>

## Function `create_currency`

Create a new currency with TreasuryCap for minting control and return the
TreasuryCap and the Metadata object. Callers may transfer/freeze the
returned objects as appropriate for their use-case.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_create_currency">create_currency</a>&lt;T: drop&gt;(witness: T, decimals: u8, symbol_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, name_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, description_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, icon_url: <a href="dependencies/move-stdlib/option.md#0x1_option_Option">option::Option</a>&lt;<a href="url.md#0x2_url_Url">url::Url</a>&gt;, ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>): (<a href="coin.md#0x2_coin_TreasuryCap">coin::TreasuryCap</a>&lt;T&gt;, <a href="coin.md#0x2_coin_CoinMetadata">coin::CoinMetadata</a>&lt;T&gt;)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_create_currency">create_currency</a>&lt;T: drop&gt;(
    witness: T,
    decimals: u8,
    symbol_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    name_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    description_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    icon_url: <a href="dependencies/move-stdlib/option.md#0x1_option_Option">option::Option</a>&lt;<a href="url.md#0x2_url_Url">url::Url</a>&gt;,
    ctx: &<b>mut</b> TxContext,
): (<a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt;, <a href="coin.md#0x2_coin_CoinMetadata">CoinMetadata</a>&lt;T&gt;) {
    // 1. Consume the witness type
    <b>let</b> _ = witness;

    // Basic safety checks for decimals
    <b>assert</b>!(decimals &lt;= 27, <a href="coin.md#0x2_coin_EINVALID_DECIMALS">EINVALID_DECIMALS</a>);

    // Convert byte literals into <a href="dependencies/move-stdlib/string.md#0x1_string">string</a> types
    <b>let</b> symbol = <a href="dependencies/move-stdlib/ascii.md#0x1_ascii_string">ascii::string</a>(symbol_bytes);
    <b>let</b> name = <a href="dependencies/move-stdlib/string.md#0x1_string_utf8">string::utf8</a>(name_bytes);
    <b>let</b> description = <a href="dependencies/move-stdlib/string.md#0x1_string_utf8">string::utf8</a>(description_bytes);

    // 2. Create the Capability and Metadata, explicitly specifying the generic type T
    <b>let</b> treasury_cap = <a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt; { id: <a href="object.md#0x2_object_new">object::new</a>(ctx), total_supply: 0 };
    <b>let</b> metadata = <a href="coin.md#0x2_coin_CoinMetadata">CoinMetadata</a>&lt;T&gt; {
        id: <a href="object.md#0x2_object_new">object::new</a>(ctx),
        decimals,
        name,
        symbol,
        description,
        icon_url
    };

    // Return the newly-created capability and metadata.
    (treasury_cap, metadata)
}
</code></pre>



</details>

<a name="0x2_coin_create_regulated_currency"></a>

## Function `create_regulated_currency`

Create a regulated currency (compatibility with kanari): returns a treasury capability,
a deny-capability for administration of a deny-list, and the metadata object.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_create_regulated_currency">create_regulated_currency</a>&lt;T: drop&gt;(witness: T, decimals: u8, symbol_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, name_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, description_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, icon_url: <a href="dependencies/move-stdlib/option.md#0x1_option_Option">option::Option</a>&lt;<a href="url.md#0x2_url_Url">url::Url</a>&gt;, ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>): (<a href="coin.md#0x2_coin_TreasuryCap">coin::TreasuryCap</a>&lt;T&gt;, <a href="deny_list.md#0x2_deny_list_DenyCap">deny_list::DenyCap</a>&lt;T&gt;, <a href="coin.md#0x2_coin_CoinMetadata">coin::CoinMetadata</a>&lt;T&gt;)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_create_regulated_currency">create_regulated_currency</a>&lt;T: drop&gt;(
    witness: T,
    decimals: u8,
    symbol_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    name_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    description_bytes: <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    icon_url: <a href="dependencies/move-stdlib/option.md#0x1_option_Option">option::Option</a>&lt;<a href="url.md#0x2_url_Url">url::Url</a>&gt;,
    ctx: &<b>mut</b> TxContext,
): (<a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt;, kanari_system::deny_list::DenyCap&lt;T&gt;, <a href="coin.md#0x2_coin_CoinMetadata">CoinMetadata</a>&lt;T&gt;) {
    <b>let</b> _ = witness;
    <b>assert</b>!(decimals &lt;= 27, <a href="coin.md#0x2_coin_EINVALID_DECIMALS">EINVALID_DECIMALS</a>);

    <b>let</b> symbol = <a href="dependencies/move-stdlib/ascii.md#0x1_ascii_string">ascii::string</a>(symbol_bytes);
    <b>let</b> name = <a href="dependencies/move-stdlib/string.md#0x1_string_utf8">string::utf8</a>(name_bytes);
    <b>let</b> description = <a href="dependencies/move-stdlib/string.md#0x1_string_utf8">string::utf8</a>(description_bytes);

    <b>let</b> treasury_cap = <a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt; { id: <a href="object.md#0x2_object_new">object::new</a>(ctx), total_supply: 0 };
    <b>let</b> denycap = kanari_system::deny_list::new_denycap&lt;T&gt;(ctx);
    <b>let</b> metadata = <a href="coin.md#0x2_coin_CoinMetadata">CoinMetadata</a>&lt;T&gt; {
        id: <a href="object.md#0x2_object_new">object::new</a>(ctx),
        decimals,
        name,
        symbol,
        description,
        icon_url
    };
    (treasury_cap, denycap, metadata)
}
</code></pre>



</details>

<a name="0x2_coin_mint"></a>

## Function `mint`

Mint new coins using TreasuryCap
Returns the newly minted Coin<T>.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_mint">mint</a>&lt;T&gt;(cap: &<b>mut</b> <a href="coin.md#0x2_coin_TreasuryCap">coin::TreasuryCap</a>&lt;T&gt;, amount: u64, ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>): <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_mint">mint</a>&lt;T&gt;(
    cap: &<b>mut</b> <a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt;,
    amount: u64,
    ctx: &<b>mut</b> TxContext,
): <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt; {
    <b>assert</b>!(amount &gt; 0, <a href="coin.md#0x2_coin_EZERO_AMOUNT">EZERO_AMOUNT</a>);
    <b>let</b> new_total = cap.total_supply + amount;
    <b>assert</b>!(new_total &gt;= cap.total_supply, <a href="coin.md#0x2_coin_EOVERFLOW">EOVERFLOW</a>);
    cap.total_supply = new_total;
    <a href="object.md#0x2_object_save_object">object::save_object</a>(cap);
    <a href="coin.md#0x2_coin_Coin">Coin</a> {
        id: <a href="object.md#0x2_object_new">object::new</a>(ctx),
        <a href="balance.md#0x2_balance">balance</a>: kanari_system::balance::create&lt;T&gt;(amount),
    }
}
</code></pre>



</details>

<a name="0x2_coin_mint_and_transfer"></a>

## Function `mint_and_transfer`

Mint and transfer to recipient


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_mint_and_transfer">mint_and_transfer</a>&lt;T&gt;(cap: &<b>mut</b> <a href="coin.md#0x2_coin_TreasuryCap">coin::TreasuryCap</a>&lt;T&gt;, amount: u64, recipient: <b>address</b>, ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_mint_and_transfer">mint_and_transfer</a>&lt;T&gt;(
    cap: &<b>mut</b> <a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt;,
    amount: u64,
    recipient: <b>address</b>,
    ctx: &<b>mut</b> TxContext,
) {
    <b>let</b> <a href="coin.md#0x2_coin">coin</a> = <a href="coin.md#0x2_coin_mint">mint</a>(cap, amount, ctx);
    <a href="transfer.md#0x2_transfer_public_transfer">transfer::public_transfer</a>(<a href="coin.md#0x2_coin">coin</a>, recipient);
}
</code></pre>



</details>

<a name="0x2_coin_burn"></a>

## Function `burn`

Burn coins, decreasing total supply


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_burn">burn</a>&lt;T&gt;(cap: &<b>mut</b> <a href="coin.md#0x2_coin_TreasuryCap">coin::TreasuryCap</a>&lt;T&gt;, <a href="coin.md#0x2_coin">coin</a>: <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;): u64
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_burn">burn</a>&lt;T&gt;(cap: &<b>mut</b> <a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt;, <a href="coin.md#0x2_coin">coin</a>: <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;): u64 {
    <b>let</b> <a href="coin.md#0x2_coin_Coin">Coin</a> { id: _, <a href="balance.md#0x2_balance">balance</a> } = <a href="coin.md#0x2_coin">coin</a>;
    <b>let</b> value = kanari_system::balance::destroy&lt;T&gt;(<a href="balance.md#0x2_balance">balance</a>);
    <b>assert</b>!(cap.total_supply &gt;= value, <a href="coin.md#0x2_coin_EUNDERFLOW">EUNDERFLOW</a>);
    cap.total_supply = cap.total_supply - value;
    <a href="object.md#0x2_object_save_object">object::save_object</a>(cap);
    value
}
</code></pre>



</details>

<a name="0x2_coin_into_balance"></a>

## Function `into_balance`

Convert a <code><a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;</code> into its inner <code>Balance&lt;T&gt;</code>.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_into_balance">into_balance</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;): <a href="balance.md#0x2_balance_Balance">balance::Balance</a>&lt;T&gt;
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_into_balance">into_balance</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;): Balance&lt;T&gt; {
    <b>let</b> <a href="coin.md#0x2_coin_Coin">Coin</a> { id: _, <a href="balance.md#0x2_balance">balance</a> } = <a href="coin.md#0x2_coin">coin</a>;
    <a href="balance.md#0x2_balance">balance</a>
}
</code></pre>



</details>

<a name="0x2_coin_from_balance"></a>

## Function `from_balance`

Construct a <code><a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;</code> from a <code>Balance&lt;T&gt;</code>.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_from_balance">from_balance</a>&lt;T&gt;(<a href="balance.md#0x2_balance">balance</a>: <a href="balance.md#0x2_balance_Balance">balance::Balance</a>&lt;T&gt;, ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>): <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_from_balance">from_balance</a>&lt;T&gt;(<a href="balance.md#0x2_balance">balance</a>: Balance&lt;T&gt;, ctx: &<b>mut</b> TxContext): <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt; {
    <a href="coin.md#0x2_coin_Coin">Coin</a> {
        id: <a href="object.md#0x2_object_new">object::new</a>(ctx),
        <a href="balance.md#0x2_balance">balance</a>
    }
}
</code></pre>



</details>

<a name="0x2_coin_total_supply"></a>

## Function `total_supply`

Get total supply from TreasuryCap


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_total_supply">total_supply</a>&lt;T&gt;(cap: &<a href="coin.md#0x2_coin_TreasuryCap">coin::TreasuryCap</a>&lt;T&gt;): u64
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_total_supply">total_supply</a>&lt;T&gt;(cap: &<a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt;): u64 {
    cap.total_supply
}
</code></pre>



</details>

<a name="0x2_coin_value"></a>

## Function `value`

Get coin value


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_value">value</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: &<a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;): u64
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_value">value</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: &<a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;): u64 {
    kanari_system::balance::value(&<a href="coin.md#0x2_coin">coin</a>.<a href="balance.md#0x2_balance">balance</a>)
}
</code></pre>



</details>

<a name="0x2_coin_zero"></a>

## Function `zero`

Create a zero-value Coin<T> (useful for testing and as a starting point)


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_zero">zero</a>&lt;T&gt;(ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>): <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_zero">zero</a>&lt;T&gt;(ctx: &<b>mut</b> TxContext): <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt; {
    <a href="coin.md#0x2_coin_Coin">Coin</a> {
        id: <a href="object.md#0x2_object_new">object::new</a>(ctx),
        <a href="balance.md#0x2_balance">balance</a>: kanari_system::balance::create&lt;T&gt;(0),
    }
}
</code></pre>



</details>

<a name="0x2_coin_split"></a>

## Function `split`

Split a coin into two. Returns the new coin with the specified amount.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_split">split</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: &<b>mut</b> <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;, amount: u64, ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>): <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_split">split</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: &<b>mut</b> <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;, amount: u64, ctx: &<b>mut</b> TxContext): <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt; {
    <b>let</b> new_balance = kanari_system::balance::split(&<b>mut</b> <a href="coin.md#0x2_coin">coin</a>.<a href="balance.md#0x2_balance">balance</a>, amount);
    <a href="object.md#0x2_object_save_object">object::save_object</a>(<a href="coin.md#0x2_coin">coin</a>);
    <a href="coin.md#0x2_coin_Coin">Coin</a> {
        id: <a href="object.md#0x2_object_new">object::new</a>(ctx),
        <a href="balance.md#0x2_balance">balance</a>: new_balance,
    }
}
</code></pre>



</details>

<a name="0x2_coin_join"></a>

## Function `join`

Join two coins together (adds the balance of 'other' into 'coin').


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_join">join</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: &<b>mut</b> <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;, other: <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_join">join</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: &<b>mut</b> <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;, other: <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;) {
    <b>let</b> <a href="coin.md#0x2_coin_Coin">Coin</a> { id: _, <a href="balance.md#0x2_balance">balance</a> } = other;
    kanari_system::balance::merge(&<b>mut</b> <a href="coin.md#0x2_coin">coin</a>.<a href="balance.md#0x2_balance">balance</a>, <a href="balance.md#0x2_balance">balance</a>);
    <a href="object.md#0x2_object_save_object">object::save_object</a>(<a href="coin.md#0x2_coin">coin</a>);
}
</code></pre>



</details>

<a name="0x2_coin_join_entry"></a>

## Function `join_entry`

Entry wrapper for joining two coin objects owned by the sender.


<pre><code><b>public</b> entry <b>fun</b> <a href="coin.md#0x2_coin_join_entry">join_entry</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: &<b>mut</b> <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;, other: <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> entry <b>fun</b> <a href="coin.md#0x2_coin_join_entry">join_entry</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: &<b>mut</b> <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;, other: <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;) {
    <a href="coin.md#0x2_coin_join">join</a>(<a href="coin.md#0x2_coin">coin</a>, other);
}
</code></pre>



</details>

<a name="0x2_coin_destroy_zero"></a>

## Function `destroy_zero`

Destroy a zero-balance coin. This function can only be called on coins with 0 balance.
Useful for cleaning up empty coin objects to save storage.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_destroy_zero">destroy_zero</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_destroy_zero">destroy_zero</a>&lt;T&gt;(<a href="coin.md#0x2_coin">coin</a>: <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;) {
    <b>let</b> <a href="coin.md#0x2_coin_Coin">Coin</a> { id: _, <a href="balance.md#0x2_balance">balance</a> } = <a href="coin.md#0x2_coin">coin</a>;
    <b>assert</b>!(kanari_system::balance::value(&<a href="balance.md#0x2_balance">balance</a>) == 0, <a href="coin.md#0x2_coin_EZERO_AMOUNT">EZERO_AMOUNT</a>);
    kanari_system::balance::destroy&lt;T&gt;(<a href="balance.md#0x2_balance">balance</a>);
}
</code></pre>



</details>

<a name="0x2_coin_divide_into_n"></a>

## Function `divide_into_n`

Split coin <code>self</code> into <code>n - 1</code> coins with equal balances. The remainder is left in
<code>self</code>. Return newly created coins.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_divide_into_n">divide_into_n</a>&lt;T&gt;(self: &<b>mut</b> <a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;, n: u64, ctx: &<b>mut</b> <a href="tx_context.md#0x2_tx_context_TxContext">tx_context::TxContext</a>): <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;<a href="coin.md#0x2_coin_Coin">coin::Coin</a>&lt;T&gt;&gt;
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_divide_into_n">divide_into_n</a>&lt;T&gt;(
    self: &<b>mut</b> <a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;, n: u64, ctx: &<b>mut</b> TxContext
): <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;<a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;&gt; {
    <b>assert</b>!(n &gt; 0, <a href="coin.md#0x2_coin_EInvalidArg">EInvalidArg</a>);
    <b>assert</b>!(n &lt;= <a href="coin.md#0x2_coin_value">value</a>(self), <a href="coin.md#0x2_coin_ENotEnough">ENotEnough</a>);

    <b>let</b> vec = <a href="dependencies/move-stdlib/vector.md#0x1_vector_empty">vector::empty</a>&lt;<a href="coin.md#0x2_coin_Coin">Coin</a>&lt;T&gt;&gt;();
    <b>let</b> i = 0;
    <b>let</b> split_amount = <a href="coin.md#0x2_coin_value">value</a>(self) / n;
    <b>while</b> (i &lt; n - 1) {
        <a href="dependencies/move-stdlib/vector.md#0x1_vector_push_back">vector::push_back</a>(&<b>mut</b> vec, <a href="coin.md#0x2_coin_split">split</a>(self, split_amount, ctx));
        i = i + 1;
    };
    vec
}
</code></pre>



</details>

<a name="0x2_coin_update_icon_url"></a>

## Function `update_icon_url`

Update the icon URL for the given coin type.
Only the holder of the TreasuryCap can perform this action.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_update_icon_url">update_icon_url</a>&lt;T&gt;(_treasury: &<a href="coin.md#0x2_coin_TreasuryCap">coin::TreasuryCap</a>&lt;T&gt;, metadata: &<b>mut</b> <a href="coin.md#0x2_coin_CoinMetadata">coin::CoinMetadata</a>&lt;T&gt;, <a href="url.md#0x2_url">url</a>: <a href="dependencies/move-stdlib/option.md#0x1_option_Option">option::Option</a>&lt;<a href="url.md#0x2_url_Url">url::Url</a>&gt;)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_update_icon_url">update_icon_url</a>&lt;T&gt;(
    _treasury: &<a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt;,
    metadata: &<b>mut</b> <a href="coin.md#0x2_coin_CoinMetadata">CoinMetadata</a>&lt;T&gt;,
    <a href="url.md#0x2_url">url</a>: <a href="dependencies/move-stdlib/option.md#0x1_option_Option">option::Option</a>&lt;<a href="url.md#0x2_url_Url">url::Url</a>&gt;
) {
    metadata.icon_url = <a href="url.md#0x2_url">url</a>;
}
</code></pre>



</details>

<a name="0x2_coin_update_name"></a>

## Function `update_name`

Update the name for the given coin type.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_update_name">update_name</a>&lt;T&gt;(_treasury: &<a href="coin.md#0x2_coin_TreasuryCap">coin::TreasuryCap</a>&lt;T&gt;, metadata: &<b>mut</b> <a href="coin.md#0x2_coin_CoinMetadata">coin::CoinMetadata</a>&lt;T&gt;, name: <a href="dependencies/move-stdlib/string.md#0x1_string_String">string::String</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_update_name">update_name</a>&lt;T&gt;(
    _treasury: &<a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt;,
    metadata: &<b>mut</b> <a href="coin.md#0x2_coin_CoinMetadata">CoinMetadata</a>&lt;T&gt;,
    name: <a href="dependencies/move-stdlib/string.md#0x1_string_String">string::String</a>
) {
    metadata.name = name;
}
</code></pre>



</details>

<a name="0x2_coin_update_symbol"></a>

## Function `update_symbol`

Update the symbol for the given coin type.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_update_symbol">update_symbol</a>&lt;T&gt;(_treasury: &<a href="coin.md#0x2_coin_TreasuryCap">coin::TreasuryCap</a>&lt;T&gt;, metadata: &<b>mut</b> <a href="coin.md#0x2_coin_CoinMetadata">coin::CoinMetadata</a>&lt;T&gt;, symbol: <a href="dependencies/move-stdlib/ascii.md#0x1_ascii_String">ascii::String</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_update_symbol">update_symbol</a>&lt;T&gt;(
    _treasury: &<a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt;,
    metadata: &<b>mut</b> <a href="coin.md#0x2_coin_CoinMetadata">CoinMetadata</a>&lt;T&gt;,
    symbol: <a href="dependencies/move-stdlib/ascii.md#0x1_ascii_String">ascii::String</a>
) {
    metadata.symbol = symbol;
}
</code></pre>



</details>

<a name="0x2_coin_update_description"></a>

## Function `update_description`

Update the description for the given coin type.


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_update_description">update_description</a>&lt;T&gt;(_treasury: &<a href="coin.md#0x2_coin_TreasuryCap">coin::TreasuryCap</a>&lt;T&gt;, metadata: &<b>mut</b> <a href="coin.md#0x2_coin_CoinMetadata">coin::CoinMetadata</a>&lt;T&gt;, description: <a href="dependencies/move-stdlib/string.md#0x1_string_String">string::String</a>)
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="coin.md#0x2_coin_update_description">update_description</a>&lt;T&gt;(
    _treasury: &<a href="coin.md#0x2_coin_TreasuryCap">TreasuryCap</a>&lt;T&gt;,
    metadata: &<b>mut</b> <a href="coin.md#0x2_coin_CoinMetadata">CoinMetadata</a>&lt;T&gt;,
    description: <a href="dependencies/move-stdlib/string.md#0x1_string_String">string::String</a>
) {
    metadata.description = description;
}
</code></pre>



</details>


[//]: # ("File containing references which can be used from documentation")

[Move Language]: https://github.com/move-language/move
[Kanari]: https://github.com/jamesatomc/kanari-cp
[Move Book]: https://move-language.github.io/move/
[Transfer Module]: transfer.md
