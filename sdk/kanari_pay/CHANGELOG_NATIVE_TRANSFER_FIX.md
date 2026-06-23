# Native KANARI transfer fix

Native KANARI transfers now use `kanari_submitTransaction` and the canonical account-balance transaction representation.

The previous SDK path called `0x2::kanari::transfer_amount` with a mutable Coin object. Gas was accounted in the account balance while the Coin object retained its pre-gas value. Recomputing the account from that object during a later transfer could restore the prior gas debit and trigger a native supply overcount.

Custom-token transfers continue to use their Move Coin objects.
