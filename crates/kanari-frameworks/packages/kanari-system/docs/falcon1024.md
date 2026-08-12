
<a name="0x2_falcon1024"></a>

# Module `0x2::falcon1024`

FIPS 206 FN-DSA-1024 / Falcon-1024 verification.


-  [Constants](#@Constants_0)
-  [Function `public_key_length`](#0x2_falcon1024_public_key_length)
-  [Function `max_signature_length`](#0x2_falcon1024_max_signature_length)
-  [Function `verify`](#0x2_falcon1024_verify)
-  [Function `rust_vector_message`](#0x2_falcon1024_rust_vector_message)
-  [Function `rust_vector_wrong_message`](#0x2_falcon1024_rust_vector_wrong_message)
-  [Function `rust_vector_public_key`](#0x2_falcon1024_rust_vector_public_key)
-  [Function `rust_vector_signature`](#0x2_falcon1024_rust_vector_signature)


<pre><code></code></pre>



<a name="@Constants_0"></a>

## Constants


<a name="0x2_falcon1024_PUBLIC_KEY_LENGTH"></a>



<pre><code><b>const</b> <a href="falcon1024.md#0x2_falcon1024_PUBLIC_KEY_LENGTH">PUBLIC_KEY_LENGTH</a>: u64 = 1793;
</code></pre>



<a name="0x2_falcon1024_MAX_SIGNATURE_LENGTH"></a>



<pre><code><b>const</b> <a href="falcon1024.md#0x2_falcon1024_MAX_SIGNATURE_LENGTH">MAX_SIGNATURE_LENGTH</a>: u64 = 2048;
</code></pre>



<a name="0x2_falcon1024_public_key_length"></a>

## Function `public_key_length`



<pre><code><b>public</b> <b>fun</b> <a href="falcon1024.md#0x2_falcon1024_public_key_length">public_key_length</a>(): u64
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="falcon1024.md#0x2_falcon1024_public_key_length">public_key_length</a>(): u64 { <a href="falcon1024.md#0x2_falcon1024_PUBLIC_KEY_LENGTH">PUBLIC_KEY_LENGTH</a> }
</code></pre>



</details>

<a name="0x2_falcon1024_max_signature_length"></a>

## Function `max_signature_length`



<pre><code><b>public</b> <b>fun</b> <a href="falcon1024.md#0x2_falcon1024_max_signature_length">max_signature_length</a>(): u64
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>public</b> <b>fun</b> <a href="falcon1024.md#0x2_falcon1024_max_signature_length">max_signature_length</a>(): u64 { <a href="falcon1024.md#0x2_falcon1024_MAX_SIGNATURE_LENGTH">MAX_SIGNATURE_LENGTH</a> }
</code></pre>



</details>

<a name="0x2_falcon1024_verify"></a>

## Function `verify`



<pre><code><b>public</b> <b>fun</b> <a href="falcon1024.md#0x2_falcon1024_verify">verify</a>(signature: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, public_key: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, message: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;): bool
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>native</b> <b>public</b> <b>fun</b> <a href="falcon1024.md#0x2_falcon1024_verify">verify</a>(signature: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, public_key: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;, message: &<a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;): bool;
</code></pre>



</details>

<a name="0x2_falcon1024_rust_vector_message"></a>

## Function `rust_vector_message`



<pre><code><b>fun</b> <a href="falcon1024.md#0x2_falcon1024_rust_vector_message">rust_vector_message</a>(): <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>fun</b> <a href="falcon1024.md#0x2_falcon1024_rust_vector_message">rust_vector_message</a>(): <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt; { x"6b616e617269206d6f76652070716320706f73697469766520766563746f72" }
</code></pre>



</details>

<a name="0x2_falcon1024_rust_vector_wrong_message"></a>

## Function `rust_vector_wrong_message`



<pre><code><b>fun</b> <a href="falcon1024.md#0x2_falcon1024_rust_vector_wrong_message">rust_vector_wrong_message</a>(): <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>fun</b> <a href="falcon1024.md#0x2_falcon1024_rust_vector_wrong_message">rust_vector_wrong_message</a>(): <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt; { x"6b616e617269206d6f76652070716320706f73697469766520766563746f7200" }
</code></pre>



</details>

<a name="0x2_falcon1024_rust_vector_public_key"></a>

## Function `rust_vector_public_key`



<pre><code><b>fun</b> <a href="falcon1024.md#0x2_falcon1024_rust_vector_public_key">rust_vector_public_key</a>(): <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>fun</b> <a href="falcon1024.md#0x2_falcon1024_rust_vector_public_key">rust_vector_public_key</a>(): <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt; { x"0a8d5e72d896a3fe947c0733f794931132fb3b0166cfb2c035125faf0b67c2c5cb8980ca135c8d937414e644ea9d185660608d29c7a8019e1aa64d81d7dba40d9688082a5d0320163092d92cc2d972551a06548c256c01e13b9dc1a18cb1899868075b7552dfbbc058df77c8e7947ee0400cca7c97978e125598a7b8fcdac783280ca8444ea97476cd637791b12f3535d12c17db142994468d109d9178e4e8da80719815127a6d5def5d257e945707a0b624e39a73e461a930dce1e3c67d8282fb638da855a3a5e098a019ed2df9556bdc07f8041154b09305b00d5174e7ed2ff9a910692a6cae2d6bd6b034088ac42400f4c9159c3d3e31444898a762ad069a9b11183f49fd8c6ad584d8584eebc5ea21788dc8f5a0d10297847a0dd4bdcb454808a474c11aa423beac2b2dd0ce280d8c976b0add65d4982be053a03fcd88aa8b2c762662ebc7292d1d8ca62ac4b0dee19ce867478c14e14e4d5013fea509696c96a96acb0d5dd156975d05367eec3debec924b4980a5888f1e5a6ee0a6aa46ae4318ec549b80d54c6122ffd5742449754104813a80be409e48e0ed910b539182ba509dba7885c0dbbe53d233d48e8b690632874eaf79395879b621f5b722deba7938071279ce533465cc1ba60e4721913e495c24b93ea8f1d17b6552a65c65072a98dc9e027157f119fc6ff1df81a847bcabbd9efae21441a9b1d7fbd9849aaf2c32f58905615549c5da39cba474ccf0d48c82e545fe78f3ccc97fb236e6d4ee6f503be079d4b2ef0880c0a0127242b84fc0c51507a27fb722ac1537419e7091034b1456a84779d82593aeb8a22d74329e245d4f6901e9dc45e31242e129776ab3750167008d4f2007de133944ecc47c8db3575025c5bb61cb01485e45ba6582f65a54980612feb81299556335e6b18b50a33805f747e990d588f4b4f9851d8921dc7064c94aa90de0b12a151fc3043a4ac86544de442c2bb16a0f439d9ddf018723e138b93ad1d36b747514ec049a85805a38033b92c59cbaf9b50a24a0b6be2835b5a2a0ce61344b4917da897f58e0014822873ef2ed29e113db04ba80d840a5b176b5e9867fa50a9989bdd08e9efb0a006542a2e71d261d025b87e0675281bed79413363950c5b30eaf219c91bd9a2c247f174cc34275ea050ab09a36392b3758ad381007dc71615d3c31ec0264565c5288d7ce9b114e35abf766c5bf691c62b0c21aa27a81b99dddddb9d44bb0b70c17742448c0c7a411485556a6e297a4abf65b715aa400627245391d52bc1925d8e9d350eb267d48299d8b760f7d3a4403895533a923bb25091795a9c78a6e65d55c14e093ab4364613efbe4a70fb99c88c951512475880428c44cc26288b15ba2320f674b60b4d0a4a0fc579a3be346a04227784cab24cd4ab098f93a6990f267d067c01a247272a9fa27252fc610f9990991e0b94a4a8d86fd4db91690a938f79fafbcad45fb39699e59eef8f6e080405fa517055529b2b405a7344f62605dbc8982da433cbd44ee596fce0d89f8bcf9bbdde064e5b0c5bfca42a901449a6155273c41272383e69200c4dd2245219115d283967561662cc664555ba6cc1f35d451e76ca8abce1c8bd8af019321c69389d68253f89a80c0d7a94ecae5e028d3f7afea615bc8af438b2233a51a68fd3b24d77b4752d5904a8401f99164840109817800e514b9f7c31daee3113dd05b6413554804d665032fd06f22314102494c22d655a29256013d108f9789d814329a40d34518c0502ac7d9e024401cae33558288438786ac630dd3bc2deadd202c5f5c76a4f12a031261103819d6348d6c3b34b498b2e4e536a2ecfa45e18221b13f99be4e591ed2a96a0153bf3472e047cb951f1c6d3d93ef90adb2e5bd043a258b5754d4f462d198b22813313688db6260f0297a859b25b5a9f3db0d2b9f62047b0f5ea47b4d6deaede59292a0e640dd49b60860ec77b957760e466562a6a9f80b61f292f863667ac82dd362599e7640a9498463b9bd78c108f4c6d8049092678fcba19e944de11bd4038d26ca0613456152a1cd373c481c98a5ae812c57207235c2132329337830a9a8a006c417c24cb979132e8eb228546ddf360df9f6b9e1848685c09609f7447d1cd4d7954e113d9b22c7d3cce7dba5961322e29dc06979d3b2e929162cae20b7074456423d4833686028053920eac4e79f642a3f534398d390f0cd982e99fe87da6b140ee6645778d0a109a74ab5d1ff6224879bb0edd05a1e96be1435f7e4e00b391d3a75b95dabac9d3185406c1174782179977161ba3fc8b13b26533e1f4afce193a81d7b34a9d067d3c85be17dd0f881c55061a3a1306b5516a1180190290f3798c3758c22ec53126646faeb108099b0554dd35a7bedfaa8af4a8d104e8fa4ed147a7321d034878d4c61516bbb92d5411226f41a5b203d21b138c4b96b2767a44b4b221fe937fac92818d02e71660aa8d657186ed117555c91cdbd41bba56b621bb15e09eaa4ee809b25bde3e24bf2a5b466e87b2050" }
</code></pre>



</details>

<a name="0x2_falcon1024_rust_vector_signature"></a>

## Function `rust_vector_signature`



<pre><code><b>fun</b> <a href="falcon1024.md#0x2_falcon1024_rust_vector_signature">rust_vector_signature</a>(): <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt;
</code></pre>



<details>
<summary>Implementation</summary>


<pre><code><b>fun</b> <a href="falcon1024.md#0x2_falcon1024_rust_vector_signature">rust_vector_signature</a>(): <a href="dependencies/move-stdlib/vector.md#0x1_vector">vector</a>&lt;u8&gt; { x"5a33d6f774ebbc57297a17f3408f9ac47d09468026dc3ea5a07270cc5341f31f378eb7e3f1447cc9e204a04d046fcf0bafab0cefb6ff7068026f63f80fd7f78fce035060ecef950daef1fae023f9109a04df6dfac0e2f3908209903df5cfd8f98fde063010f7bfec04b10ce69f6ffa109bfb8fd1f73f7e022fe7fa108cf81f04f68fdf04bf7e006fe3117fd6ff6fbc009ff912e078fd4f79025f64fa8fa519c02f06b0d7180ffcf58fdbf01062fff182020f7d0beff0f8bf90034fe3011fe20a4ee0f9ef860c0115fb5f0701d057045f34f88005051ecefdaf9d0ebf47e9dffefd6072f99fd5ff60330150e0ff0f46059fd0fe7026f7a1aefb505cf090f3f8e12bf950f70bcf6efeb0cc11d113f6ff7905004a0010cfffbf67ffb0870b1ff1f53078019045f5ff68ff7f3f07ef2af32f660e2105024ec2fecfb50ce0d6ea9127f6eef2018066f17eaff9311ef1bf85f3104af750b4fe2f74fd2fce04efa408302bfafff2fb6fa012d1e601103cf19f75ee6f300f107f06bf87f50e8f0860be0ecfe4f6104e0090230a20640b7fd703f005eca00af4ff19089fa1ff0feaf15124ff008401005cf540bcfe4f8310efd2073187f5cfee0ad067fe1fef0fb079fc302e0b806a181f5f044078f6a03a0f50bafd2ffe068fe6fadef806f015f81fc300b04d045fb3027ffe013ff111ffff09d00a084fc6fe011bfd8094ff0000134f6ef75fd7f81f66faaf90fff23007ffb7f32ed7ff3047ef0f3f0341d5068f5011f0b3f9711d1a5e88f77fbffc60c4f77fc70420360a9fcefd3f84031fedfe80d309714c13d06e19c085fd801ce91f540f3f28ff0015025084fe0f27033f8e050fe0ed1ffb02d01c09cf0c00304ced800f08110cfb901a036126e6ef3c0b8f060220ce038ffe147f50fa2fc7f5406708cef3fa0f79152f37041fc201cf5e044f9ff8205bf31094fba0500bc0a5f1b11506c018093ffa06e0f005cfeb050fdff3a063fb6ed5efbffd0a9f3bfe1f83ff3f78fe7f4e040f5a0dd012f2b046ff205cf9bfe407af66f930750b5077f8e070ee80defa5faefe6081f7b04f0b8145fa2f9effd15bf53f740b3009f7ff3cfb3fdefd6f230b708afe3fa9fb202af3a02205cfe7f58fd5f8ff46fa8078eeef06f61fb2082f090a9f4c00504804212701a06c003105ff602c0a207904c01413a0510faf38eff057fd004601f142f6c0a104a047fd705ef33fb700017efe1fce092f03f6b127f66fe60a204cfca027fc8fe7eb1ff3f95148007ff80b70ad032fab11606b194fa00791adf180c5fadfe5e8a01efb509ff8ef6d02ff8700ff62e8d059056f86ee402eff20eb0420040cc012fb6fb3f0f05df450021250c80220760dcf6cfca10ceeef9cf810a2f8304ffc60acfd600bf73020f5b0331b608100aef6f7ff560cb0adf9c050eda03feebf5bff40610680f606114afe90680750c2fbe00afb6fdb05ef87ea50dbf91fda030fc4f7607bf7e0c30bc041048fa6f51068fe0f84f6ef6102903f089046018fdcfde03af4f05008f08301dfc5028f8d051f1ef30facfc8f9710a04af0c0ccfbef5c06cfb5f2ffe7fb3fd1f5010f0700f804405eef8f86f42fe1f4405cf80faefa5e53ff10d4f18022fd8f5708bfec0b10e3f71f51ff8f61fdff8affef87fed080ed4feafab0a307df78048027fb807e00c0e9f1208cf8ff430980c2eda1170c7f2d0c8fcceb8f6b00100a07ff55fa80030ac003fe00a7fec0adfefeff05e02ff5b082fbefa7eecef901e028009e99fd8f81efc039f85f82039fb5089008f6c04608a0f1fff02bffffe5fd5fa7016f10f0505a066fc6fe30d7fb50b218f0c30681190c5f0afe1ffdf60ffe08affd108f82f88efffe1fc1f85f900d703bf9214cf58f61fa3f43f95042148f8cfc103df97eedf9df75fe4f2e0a202aea7f2601b082fbc03c120f54f84ff70c3fc8fcdf34159fd40fc007fe70b0ffb036ffafb5edceeef39fb1078169034fb705b011f680cc0fef68006ed3eb0ec3fb3154f94fcefe407701508afa5f57fe2fb5fbafb9f400b0052f8606dfe407702e09ef970a5026f6d0a9006f6f049eda0d1f3b008fde01d1d419806af6207205400e070012119f64104eb90a10c301e03f0ae036eacfea0affa4f06140f210e8019fcefedff1f0ef2f1160a9ffff2d0acf90f6605ff6814c08e0abf8b0e7f51fa804f030fd20dc034fd9f7802f0c3f82007f4b05e050" }
</code></pre>



</details>


[//]: # ("File containing references which can be used from documentation")

[Move Language]: https://github.com/move-language/move
[Kanari]: https://github.com/jamesatomc/kanari-cp
[Move Book]: https://move-language.github.io/move/
[Transfer Module]: transfer.md
