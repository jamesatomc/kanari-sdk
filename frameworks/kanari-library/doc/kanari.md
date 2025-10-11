
<a name="0x6_kanari"></a>

# Module `0x6::kanari`



-  [Resource `KARI`](#0x6_kanari_KARI)
-  [Resource `TokenAdmin`](#0x6_kanari_TokenAdmin)
-  [Constants](#@Constants_0)
-  [Function `mint`](#0x6_kanari_mint)
-  [Function `burn`](#0x6_kanari_burn)
-  [Function `transfer`](#0x6_kanari_transfer)


<pre><code><b>use</b> <a href="">0x1::option</a>;
<b>use</b> <a href="">0x1::string</a>;
<b>use</b> <a href="">0x2::object</a>;
<b>use</b> <a href="">0x2::signer</a>;
<b>use</b> <a href="">0x3::account_coin_store</a>;
<b>use</b> <a href="">0x3::coin</a>;
</code></pre>



<a name="0x6_kanari_KARI"></a>

## Resource `KARI`



<pre><code><b>struct</b> <a href="kanari.md#0x6_kanari_KARI">KARI</a> <b>has</b> store, key
</code></pre>



<a name="0x6_kanari_TokenAdmin"></a>

## Resource `TokenAdmin`



<pre><code><b>struct</b> <a href="kanari.md#0x6_kanari_TokenAdmin">TokenAdmin</a> <b>has</b> store, key
</code></pre>



<a name="@Constants_0"></a>

## Constants


<a name="0x6_kanari_DECIMALS"></a>



<pre><code><b>const</b> <a href="kanari.md#0x6_kanari_DECIMALS">DECIMALS</a>: u8 = 8;
</code></pre>



<a name="0x6_kanari_ADMIN_ADDRESS"></a>



<pre><code><b>const</b> <a href="kanari.md#0x6_kanari_ADMIN_ADDRESS">ADMIN_ADDRESS</a>: <b>address</b> = 0x6;
</code></pre>



<a name="0x6_kanari_ERROR_NOT_ADMIN"></a>



<pre><code><b>const</b> <a href="kanari.md#0x6_kanari_ERROR_NOT_ADMIN">ERROR_NOT_ADMIN</a>: u64 = 1;
</code></pre>



<a name="0x6_kanari_ERROR_ZERO_AMOUNT"></a>



<pre><code><b>const</b> <a href="kanari.md#0x6_kanari_ERROR_ZERO_AMOUNT">ERROR_ZERO_AMOUNT</a>: u64 = 2;
</code></pre>



<a name="0x6_kanari_INITIAL_SUPPLY"></a>



<pre><code><b>const</b> <a href="kanari.md#0x6_kanari_INITIAL_SUPPLY">INITIAL_SUPPLY</a>: <a href="">u256</a> = 10000000000000000;
</code></pre>



<a name="0x6_kanari_KARI_ICON_URL"></a>



<pre><code><b>const</b> <a href="kanari.md#0x6_kanari_KARI_ICON_URL">KARI_ICON_URL</a>: <a href="">vector</a>&lt;u8&gt; = [];
</code></pre>



<a name="0x6_kanari_mint"></a>

## Function `mint`



<pre><code><b>public</b> entry <b>fun</b> <a href="kanari.md#0x6_kanari_mint">mint</a>(admin: &<a href="">signer</a>, <b>to</b>: <b>address</b>, amount: <a href="">u256</a>)
</code></pre>



<a name="0x6_kanari_burn"></a>

## Function `burn`



<pre><code><b>public</b> entry <b>fun</b> <a href="kanari.md#0x6_kanari_burn">burn</a>(<a href="">account</a>: &<a href="">signer</a>, amount: <a href="">u256</a>)
</code></pre>



<a name="0x6_kanari_transfer"></a>

## Function `transfer`



<pre><code><b>public</b> entry <b>fun</b> <a href="">transfer</a>(from: &<a href="">signer</a>, <b>to</b>: <b>address</b>, amount: <a href="">u256</a>)
</code></pre>
