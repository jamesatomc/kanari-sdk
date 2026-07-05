// modules/transactions/constants.dart
// Transaction module constants.

class TransactionConstants {
  const TransactionConstants._();

  // Backend-native KANARI balance calls mirrored by Transaction::native_call.
  // These are not Move entry functions; RPC rebuilds and verifies the same signed Transaction.
  static const String nativeKanariModule = '0x2::kanari';
  static const String nativeBurnAmountFunction = 'burn_amount';

  // RPC methods
  static const String rpcPublishModule = 'kanari_publishModule';
  static const String rpcSubmitTransaction = 'kanari_submitTransaction';
  static const String rpcCallFunction = 'kanari_callFunction';

  // Default gas settings
  static const int defaultGasLimit = 100000;
  static const int defaultGasPrice = 1000;
}
