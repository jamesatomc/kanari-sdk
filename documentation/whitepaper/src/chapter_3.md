# 3. Move Execution and Object Model

## 3.1 Deterministic execution

Move modules express typed resources and application state transitions. Ordered transactions execute deterministically and produce a changeset. Failed execution is atomic: no partial canonical state is accepted.

## 3.2 Owned and shared objects

Independent owned-object transactions can be scheduled in parallel. Conflicting access sets, shared objects, and hot objects remain dependency-ordered. Object versions and digests prevent stale references from being reused.

Native `Coin<KANARI>` inputs have strict runtime ownership. A mutable transfer coin must be distinct from the gas object, and a sender cannot mutate another owner's coin. DeFi objects may be passed explicitly across owners for escrow workflows; the Move contract must enforce buyer, seller, admin, and owner roles.

## 3.3 Gas metering

Gas accounts for bytecode, serialization, vectors, native calls, and value size. Native transfer, split, merge, burn, and gas adjustment paths share the same accounting and fail closed on overflow or insufficient balance.

## 3.4 Developer workflow

Developers build and test Move packages, publish modules through the Kanari CLI, submit signed transactions through RPC, and inspect committed effects and object references.

## 3.5 Access-set scheduling

The scheduler classifies inputs as owned, shared, immutable, or gas objects. Independent owned-object transactions may run in parallel; transactions touching the same versioned object are ordered through a dependency lane. Parallel execution must remain equivalent to deterministic serial replay.

## 3.6 DeFi and escrow obligations

Runtime acceptance of a mutable object does not prove business authority. An escrow module must check buyer, seller, administrator, state transition, amount, and replay conditions on every public entry function. Tests must cover create, delivery, release, dispute, refund, and unauthorized cross-role calls.

## 3.7 Failure atomicity

Effects are staged as a changeset. Validation covers ownership, versions, duplicate mutable inputs, supply arithmetic, and gas. The applier commits an accepted set atomically from the perspective of canonical state; recovery must never expose a half-applied object graph.

## 3.8 Conflict rule

For transactions `T_i` and `T_j`, parallel execution is valid only when their mutable access sets are disjoint:

`Mutable(T_i) ∩ Mutable(T_j) = ∅`

If the intersection is non-empty, the scheduler must impose a dependency order or reject the stale reference. This is a scheduling condition, not a replacement for Move-level authorization.
