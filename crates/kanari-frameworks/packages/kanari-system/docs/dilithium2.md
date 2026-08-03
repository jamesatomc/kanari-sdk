
<a name="0x2_dilithium2"></a>

# Module `0x2::dilithium2`

NIST ML-DSA / Dilithium2 verification.


-  [Constants](#@Constants_0)
-  [Function `public_key_length`](#0x2_dilithium2_public_key_length)
-  [Function `signature_length`](#0x2_dilithium2_signature_length)
-  [Function `verify`](#0x2_dilithium2_verify)


<pre><code></code></pre>



<a name="@Constants_0"></a>

## Constants


<a name="0x2_dilithium2_PUBLIC_KEY_LENGTH"></a>



<pre><code><b>const</b> <a href="dilithium2.md#0x2_dilithium2_PUBLIC_KEY_LENGTH">PUBLIC_KEY_LENGTH</a>: u64 = 1312;
</code></pre>



<a name="0x2_dilithium2_SIGNATURE_LENGTH"></a>



<pre><code><b>const</b> <a href="dilithium2.md#0x2_dilithium2_SIGNATURE_LENGTH">SIGNATURE_LENGTH</a>: u64 = 2420;
</code></pre>



<a name="0x2_dilithium2_public_key_length"></a>

## Function `public_key_length`



<pre><code><b>public</b> <b>fun</b> <a href="dilithium2.md#0x2_dilithium2_public_key_length">public_key_length</a>(): u64
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="dilithium2.md#0x2_dilithium2_public_key_length">public_key_length</a>(): u64 { <a href="dilithium2.md#0x2_dilithium2_PUBLIC_KEY_LENGTH">PUBLIC_KEY_LENGTH</a> }
</code></pre>



</details>

<a name="0x2_dilithium2_signature_length"></a>

## Function `signature_length`



<pre><code><b>public</b> <b>fun</b> <a href="dilithium2.md#0x2_dilithium2_signature_length">signature_length</a>(): u64
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="dilithium2.md#0x2_dilithium2_signature_length">signature_length</a>(): u64 { <a href="dilithium2.md#0x2_dilithium2_SIGNATURE_LENGTH">SIGNATURE_LENGTH</a> }
</code></pre>



</details>

<a name="0x2_dilithium2_verify"></a>

## Function `verify`



<pre><code><b>public</b> <b>fun</b> <a href="dilithium2.md#0x2_dilithium2_verify">verify</a>(signature: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, public_key: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, message: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;): bool
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>native</b> <b>public</b> <b>fun</b> <a href="dilithium2.md#0x2_dilithium2_verify">verify</a>(signature: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, public_key: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, message: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;): bool;
</code></pre>



</details>


[//]: # ("File containing references which can be used from documentation")

[Move Language]: https://github.com/move-language/move
[Kanari]: https://github.com/jamesatomc/kanari-cp
[Move Book]: https://move-language.github.io/move/
[Transfer Module]: transfer.md
