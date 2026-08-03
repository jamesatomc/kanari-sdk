
<a name="0x2_k256_dilithium3"></a>

# Module `0x2::k256_dilithium3`

Hybrid K256 + Dilithium3 verification. Both components must verify.


-  [Function `verify`](#0x2_k256_dilithium3_verify)


<pre><code></code></pre>



<a name="0x2_k256_dilithium3_verify"></a>

## Function `verify`



<pre><code><b>public</b> <b>fun</b> <a href="k256_dilithium3.md#0x2_k256_dilithium3_verify">verify</a>(signature: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, k256_public_key: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, dilithium3_public_key: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, message: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;): bool
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>native</b> <b>public</b> <b>fun</b> <a href="k256_dilithium3.md#0x2_k256_dilithium3_verify">verify</a>(
    signature: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    k256_public_key: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    dilithium3_public_key: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
    message: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;,
): bool;
</code></pre>



</details>


[//]: # ("File containing references which can be used from documentation")

[Move Language]: https://github.com/move-language/move
[Kanari]: https://github.com/jamesatomc/kanari-cp
[Move Book]: https://move-language.github.io/move/
[Transfer Module]: transfer.md
