
<a name="@Kanari_System_Modules_0"></a>

# Kanari System Modules


This is the root document for the Kanari System module documentation. The Kanari System provides Move modules for blockchain operations including transfers, token management, and system utilities.


<a name="@Overview_1"></a>

## Overview


The Kanari System is built on top of the Move programming language, providing a secure and efficient platform for decentralized applications. This documentation covers all modules available in the Kanari System.


<a name="@Features_2"></a>

## Features


- **Transfer Module**: Production-ready transfer functionality with full validation
- **Type Safety**: Strong type checking and validation
- **Security**: Built-in error handling and validation
- **Performance**: Optimized for efficient execution


<a name="@Index_3"></a>

## Index


-  [`0x2::address`](address.md#0x2_address)
-  [`0x2::bag`](bag.md#0x2_bag)
-  [`0x2::balance`](balance.md#0x2_balance)
-  [`0x2::base64`](base64.md#0x2_base64)
-  [`0x2::bcs`](bcs.md#0x2_bcs)
-  [`0x2::clock`](clock.md#0x2_clock)
-  [`0x2::coin`](coin.md#0x2_coin)
-  [`0x2::collection`](collection.md#0x2_collection)
-  [`0x2::deny_list`](deny_list.md#0x2_deny_list)
-  [`0x2::dilithium2`](dilithium2.md#0x2_dilithium2)
-  [`0x2::dilithium3`](dilithium3.md#0x2_dilithium3)
-  [`0x2::dilithium5`](dilithium5.md#0x2_dilithium5)
-  [`0x2::dynamic_field`](dynamic_field.md#0x2_dynamic_field)
-  [`0x2::dynamic_object_field`](dynamic_object_field.md#0x2_dynamic_object_field)
-  [`0x2::ecdsa_k1`](ecdsa_k1.md#0x2_ecdsa_k1)
-  [`0x2::ecdsa_r1`](ecdsa_r1.md#0x2_ecdsa_r1)
-  [`0x2::ed25519`](ed25519.md#0x2_ed25519)
-  [`0x2::ed25519_dilithium3`](ed25519_dilithium3.md#0x2_ed25519_dilithium3)
-  [`0x2::event`](event.md#0x2_event)
-  [`0x2::hex`](hex.md#0x2_hex)
-  [`0x2::k256_dilithium3`](k256_dilithium3.md#0x2_k256_dilithium3)
-  [`0x2::kanari`](kanari.md#0x2_kanari)
-  [`0x2::math`](math.md#0x2_math)
-  [`0x2::multisig`](multisig.md#0x2_multisig)
-  [`0x2::object`](object.md#0x2_object)
-  [`0x2::pay`](pay.md#0x2_pay)
-  [`0x2::rs256`](rs256.md#0x2_rs256)
-  [`0x2::table`](table.md#0x2_table)
-  [`0x2::transfer`](transfer.md#0x2_transfer)
-  [`0x2::tx_context`](tx_context.md#0x2_tx_context)
-  [`0x2::url`](url.md#0x2_url)



<a name="@Getting_Started_4"></a>

## Getting Started


To use Kanari System modules in your Move code, add the following to your <code>Move.toml</code>:

```toml
[dependencies]
KanariSystem = { local = "../kanari-system" }
```

Then import the modules you need:

```move
use kanari_system::transfer;
```


<a name="@Support_5"></a>

## Support


For issues and questions, please visit our GitHub repository.


[//]: # ("File containing references which can be used from documentation")

[Move Language]: https://github.com/move-language/move
[Kanari]: https://github.com/jamesatomc/kanari-cp
[Move Book]: https://move-language.github.io/move/
[Transfer Module]: transfer.md
