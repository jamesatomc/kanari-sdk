# Kanari object-centric migration

This branch moves Kanari away from account-centric execution and toward an object-centric protocol model similar to Sui.

## Protocol rule

An address is still a signer and an owner, but it is no longer the canonical state container for balances, nonces, modules, or user resources.

Canonical execution dependencies are object references:

```text
(object_id, version, digest)
```

A transaction is replay-protected by its digest and by consuming exact object versions. It must not rely on an account sequence number.

## New canonical path

The new path introduced on this branch is:

1. `kanari-types::object`
   - `ObjectID`
   - `ObjectDigest`
   - `ObjectRef`
   - `Owner::{AddressOwner,ObjectOwner,Shared,Immutable}`

2. `kanari-types::object_transaction`
   - `ObjectTransactionData`
   - `ObjectTransactionKind`
   - `ObjectArg`
   - `GasData`

3. `kanari-types::signed_object_transaction`
   - sender signature over object inputs
   - optional sponsor signature over gas payment objects

4. `kanari-types::object_effects`
   - Lamport object versioning
   - created/mutated/deleted object effects
   - gas cost summary

5. `kanari-core::object_transaction_engine_v2`
   - persistent object transaction pool
   - mutable-object locking
   - exact object reference validation
   - explicit gas coin balance validation

6. `kanari-core::object_gas`
   - explicit gas coin charge planning
   - multiple gas coins smashed into the first gas coin
   - remaining gas coins deleted

## Legacy account-centric code to remove

These paths are still legacy and should not be expanded:

- `Transaction::{PublishModule, ExecuteFunction}` fields:
  - `sequence_number`
  - `gas_limit`
  - `gas_price`
  - implicit sender balance checks
- `StateManager::validate_sequence`
- `StateManager::apply_zero_effect_sequence_batch`
- `ChangeSet::account_changes`
- `Account::sequence_number`
- account-native balances as consensus source of truth
- runtime auto-selection or auto-merge of owner coins
- sender/sequence ordering in the legacy mempool

## Migration order

### Phase 1: object admission

Status: started.

- Accept signed `SignedObjectTransaction`.
- Verify sender and sponsor signatures.
- Validate every `ObjectRef` against current state.
- Lock mutable objects before consensus/execution.
- Reject shared objects until the consensus path is explicitly wired.

### Phase 2: object gas

Status: started.

- Charge only explicitly listed gas coin objects.
- Never scan all owner coins during execution.
- Write gas effects as mutated/deleted objects.
- Do not credit a DAO account balance through `AccountChange`.

### Phase 3: object effects application

Next.

- Add an effects applier that writes `ObjectTransactionEffectsV1` to state.
- Persist `previous_transaction` and digest as canonical object metadata.
- Delete object rows through effects, not through account deltas.
- Update owner indexes from object owner changes.

### Phase 4: Move adapter

Next.

- Convert Move writes into object writes.
- Forbid user global resources as canonical state.
- Treat packages as package objects instead of `Account.modules`.
- Keep address-based resources only for framework internals during the transition.

### Phase 5: RPC and wallet adapter

Next.

- Add RPC submit path for object transactions.
- Wallet selects coin object refs before signing.
- Sponsored transactions require gas owner signature.
- Balance APIs become derived indexes over `Coin<T>` objects.

### Phase 6: remove account-centric consensus state

Final.

- Remove account sequence validation from execution.
- Remove account balance source of truth.
- Remove implicit gas debit from sender account.
- Keep address only as signer/owner identity.

## Guardrails

- Do not silently auto-select coins inside the runtime.
- Do not mutate objects without checking `(id, version, digest)`.
- Do not use `existing.version + 1` per object; use one transaction Lamport version derived from the highest mutable input.
- Do not introduce a new per-address nonce.
- Do not treat account balance as consensus state for user funds.
