# Kanari Escrow Module

Smart contract module for escrow transactions on the Kanari blockchain.

## Overview

This module implements a secure escrow system that allows buyers and sellers to conduct transactions with built-in protection. Funds are locked in escrow until both parties confirm the transaction, or disputes can be raised if issues arise.

## Features

- 🔒 **Secure Fund Locking**: Buyer's funds are locked in escrow until deal completion
- ✅ **Two-Party Confirmation**: Both buyer and seller must confirm transaction steps
- 📝 **Proof Tracking**: All state transitions are recorded on-chain as proof entries
- ⚖️ **Dispute Resolution**: Either party can raise disputes for unresolved issues
- 🎯 **Event Emission**: Real-time events for all state changes

## Architecture

### Core Components

1. **EscrowDeal**: Main resource storing deal information and locked funds
2. **EscrowProof**: On-chain record of all state transitions
3. **ProofEntry**: Individual proof record for each state change

### Deal States

```
STATE_LOCKED (1)     → Funds locked, waiting for delivery
STATE_DELIVERED (2)  → Seller confirmed delivery
STATE_COMPLETED (3)  → Buyer confirmed, funds released to seller
STATE_DISPUTED (4)   → Dispute raised, awaiting resolution
STATE_CANCELLED (5)  → Deal cancelled, funds returned
```

## Usage

### CLI Note

When calling these entry functions through `kanari move call`, pass a non-zero gas price on networks that enforce `min_gas_price >= 1`.

Example:

```bash
cargo r -p kanari move call --gas-price 1000 --package <PACKAGE> --module usdc --function mint --args <amount> <recipient>
```

### 1. Create a Deal (Buyer)

```move
use kanari_escrow::escrow;
use kanari_system::coin;
use kanari_system::tx_context;

// Prepare coin for escrow
let escrow_coin = coin::split(&mut my_coin, amount, ctx);

// Create deal
escrow::create_deal<CoinType>(
    string::utf8(b"deal-001"),      // Unique deal ID
    seller_address,                  // Seller's address
    1000,                            // Amount in smallest unit
    string::utf8(b"Product purchase"), // Description
    escrow_coin,                     // Locked funds
    ctx                              // Transaction context
);
```

### 2. Confirm Delivery (Seller)

```move
// Seller confirms delivery
escrow::confirm_delivery<CoinType>(
    &mut deal_object,    // Mutable reference to deal
    &mut proof_object,   // Mutable reference to proof
    ctx                  // Transaction context
);
```

### 3. Release Funds (Buyer)

```move
// Buyer confirms receipt and releases funds
escrow::release_funds<CoinType>(
    &mut deal_object,    // Mutable reference to deal
    &mut proof_object,   // Mutable reference to proof
    ctx                  // Transaction context
);
```

### 4. Raise Dispute (Either Party)

```move
// Either buyer or seller can raise dispute
escrow::raise_dispute<CoinType>(
    &mut deal_object,    // Mutable reference to deal
    &mut proof_object,   // Mutable reference to proof
    ctx                  // Transaction context
);
```

### 5. View Functions

```move
// Get current deal state
let state = escrow::get_state<CoinType>(&deal_object);

// Get proof entry count
let count = escrow::get_proof_count(&proof_object);
```

## Flutter UI Integration

The Kanari Kit includes a complete Flutter UI for managing escrow transactions.

### Accessing the Escrow Screen

From the HomeScreen, tap the **security icon** (🛡️) in the top-right corner to access the Escrow screen.

### UI Features

The Escrow screen has three tabs:

#### 1. **Create Tab** - Create New Deals

- Enter deal ID (unique identifier)
- Input seller's address
- Specify amount to lock
- Add description
- Click "Create Deal & Lock Funds"

#### 2. **Actions Tab** - Perform Deal Actions

- **Seller Actions**:
  - Confirm Delivery: Mark item as delivered
- **Buyer Actions**:
  - Release Funds: Confirm receipt and release payment
  - Raise Dispute: Report issues with the transaction

#### 3. **Check Tab** - View Deal Status

- Enter buyer's address (deal owner)
- View current deal state with color-coded status
- See number of proof entries recorded

### State Visualization

| State | Color | Icon | Description |
|-------|-------|------|-------------|
| Locked | 🟠 Orange | 🔒 | Funds locked, waiting for delivery |
| Delivered | 🔵 Blue | 📦 | Seller confirmed delivery |
| Completed | 🟢 Green | ✅ | Transaction completed successfully |
| Disputed | 🔴 Red | ⚖️ | Dispute raised, needs resolution |

## Error Codes

| Code | Error | Description |
|------|-------|-------------|
| 1 | `E_NOT_BUYER` | Caller is not the buyer |
| 2 | `E_NOT_SELLER` | Caller is not the seller |
| 3 | `E_WRONG_STATE` | Invalid state for requested operation |
| 4 | `E_ALREADY_EXISTS` | Deal ID already exists |
| 5 | `E_DEAL_NOT_FOUND` | Deal does not exist |
| 6 | `E_NOT_AUTHORIZED` | Caller not authorized for this action |

## Events

### `DealCreated`

Emitted when a new escrow deal is created.

**Fields:**

- `deal_id: String`
- `buyer: address`
- `seller: address`
- `amount: u64`

### `DealStateChanged`

Emitted when deal state changes.

**Fields:**

- `deal_id: String`
- `old_state: u8`
- `new_state: u8`
- `actor: address`
- `timestamp: u64`

## Security Considerations

1. **Fund Safety**: Funds remain in escrow until both parties confirm or dispute is resolved
2. **Immutable Proof Trail**: All state changes are recorded on-chain and cannot be altered
3. **Access Control**: Only buyer/seller can perform actions on their deals
4. **State Validation**: Each operation validates current state before proceeding

## Development Notes

### Kanari System Integration

This module uses Kanari System framework instead of Aptos Framework:

- **TxContext** instead of `&signer` for transaction context
- **Timestamp Access**: Uses `tx_context::epoch_timestamp_ms(ctx)` which provides millisecond-precision timestamps from the transaction context
  - Note: The `kanari_system::clock` module exists but requires passing the Clock shared object as a parameter
  - For simplicity and better ergonomics, we use `tx_context::epoch_timestamp_ms()` which is equivalent and easier to use
- **Object-based storage** with `object::UID` instead of account-based resources
- **Transfer module** for object ownership management
- **Event emission** without `#[event]` attribute

### Timestamp Implementation

The escrow module uses a helper function `now_ms(ctx: &TxContext)` that wraps `tx_context::epoch_timestamp_ms()`:

```move
fun now_ms(ctx: &TxContext): u64 {
    tx_context::epoch_timestamp_ms(ctx)
}
```

This approach:

- ✅ Provides consistent timestamp access across all functions
- ✅ Returns milliseconds (not seconds) for higher precision
- ✅ Doesn't require managing Clock object references
- ✅ Is the recommended way to access time in Kanari Move contracts

### Flutter Integration

The Flutter UI integrates with the escrow module through:

1. **KanariClient** (`lib/src/kanari_client.dart`):
   - `createEscrowDeal()` - Create new deals
   - `confirmDelivery()` - Seller confirms delivery
   - `releaseFunds()` - Buyer releases funds
   - `raiseDispute()` - Raise disputes
   - `getDealState()` - Check deal state (view function)
   - `getProofCount()` - Get proof count (view function)

2. **EscrowScreen** (`lib/src/ui/screens/escrow_screen.dart`):
   - Three-tab interface for different operations
   - Real-time state visualization
   - Error handling and success feedback
   - Loading states during transactions

3. **BCS Serialization**:
   - All arguments are serialized using BCS format
   - Addresses encoded as 32-byte arrays
   - Strings encoded as length-prefixed UTF-8
   - U64 values encoded in little-endian format

### Testing

Run tests with:

```bash
kanari move test --package-dir example_move/kanari_kit/kanari_escrow
```

Test the Flutter UI:

```bash
cd example_move/kanari_kit
flutter run
```

## Example Workflow

### Complete Transaction Flow

1. **Buyer creates deal**:

   ```dart
   await client.createEscrowDeal(
     wallet: buyerWallet,
     dealId: 'deal-001',
     sellerAddress: '0x123...',
     amount: 1000,
     description: 'Purchase of digital art',
   );
   ```

2. **Seller confirms delivery**:

   ```dart
   await client.confirmDelivery(
     wallet: sellerWallet,
     buyerAddress: '0x456...',
   );
   ```

3. **Buyer releases funds**:

   ```dart
   await client.releaseFunds(
     wallet: buyerWallet,
     buyerAddress: '0x456...',
   );
   ```

4. **Check final state**:

   ```dart
   final state = await client.getDealState('0x456...');
   print('Deal state: $state'); // Should be 3 (Completed)
   ```

## License

Apache-2.0

## Contributing

Contributions are welcome! Please read our contributing guidelines before submitting PRs.
