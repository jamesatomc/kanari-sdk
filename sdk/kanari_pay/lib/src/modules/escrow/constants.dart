// NEW FILE: modules/escrow/constants.dart
// Escrow module constants.

class EscrowConstants {
  static const String packageAddress =
      '0x3ba63b92aac5f2bff87e580e820b61faf1c5fe9ae12f0bc8addd931a340b3146';
  static const String module = 'escrow';

  // Entry functions
  static const String fnCreateDeal = 'create_deal';
  // ID-based entry points are required for cross-owner actions. The deal and
  // proof belong to the buyer, while the seller must be able to confirm
  // delivery without being declared the object owner in the transaction.
  static const String fnConfirmDelivery = 'confirm_delivery';
  static const String fnReleaseFunds = 'release_funds';
  static const String fnRaiseDispute = 'raise_dispute';

  // View functions
  static const String fnGetState = 'get_state_ref';
  static const String fnGetDealDetails = 'get_deal_details_ref';
  static const String fnGetProofCount = 'get_proof_count';

  // Object types
  static const String objectTypeDeal = 'EscrowDeal';
  static const String objectTypeProof = 'EscrowProof';

  // State constants
  static const int stateLocked = 1;
  static const int stateDelivered = 2;
  static const int stateCompleted = 3;
  static const int stateDisputed = 4;

  /// Get state name from int
  static String getStateName(int state) {
    switch (state) {
      case stateLocked:
        return 'Locked';
      case stateDelivered:
        return 'Delivered';
      case stateCompleted:
        return 'Completed';
      case stateDisputed:
        return 'Disputed';
      default:
        return 'Unknown';
    }
  }

  /// Check if state matches expected
  static bool isState(int actual, int expected) => actual == expected;
}
