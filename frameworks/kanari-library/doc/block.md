
<a name="0x6_block"></a>

# Module `0x6::block`



-  [Resource `BlockHeader`](#0x6_block_BlockHeader)
-  [Resource `Block`](#0x6_block_Block)
-  [Constants](#@Constants_0)
-  [Function `create_block`](#0x6_block_create_block)


<pre><code><b>use</b> <a href="">0x1::vector</a>;
<b>use</b> <a href="">0x2::bcs</a>;
<b>use</b> <a href="">0x2::hash</a>;
<b>use</b> <a href="">0x2::object</a>;
<b>use</b> <a href="">0x2::signer</a>;
</code></pre>



<a name="0x6_block_BlockHeader"></a>

## Resource `BlockHeader`



<pre><code><b>struct</b> <a href="block.md#0x6_block_BlockHeader">BlockHeader</a> <b>has</b> store, key
</code></pre>



<a name="0x6_block_Block"></a>

## Resource `Block`



<pre><code><b>struct</b> <a href="block.md#0x6_block_Block">Block</a> <b>has</b> store, key
</code></pre>



<a name="@Constants_0"></a>

## Constants


<a name="0x6_block_ErrorAlreadyExists"></a>

Only admin can create blocks


<pre><code><b>const</b> <a href="block.md#0x6_block_ErrorAlreadyExists">ErrorAlreadyExists</a>: u64 = 1;
</code></pre>



<a name="0x6_block_create_block"></a>

## Function `create_block`



<pre><code><b>public</b> entry <b>fun</b> <a href="block.md#0x6_block_create_block">create_block</a>(admin: &<a href="">signer</a>, prev_hash: <a href="">vector</a>&lt;u8&gt;, merkle_root: <a href="">vector</a>&lt;u8&gt;, time: u64, nonce: u64, transactions: <a href="">vector</a>&lt;<a href="">vector</a>&lt;u8&gt;&gt;)
</code></pre>
