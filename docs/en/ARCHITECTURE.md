# Architecture

## Overview

`clob` is a deterministic central limit order book engine, structured as a
pipeline of four stages. Each command passes through the stages in order and produces
a vector of events.

```
                    ┌──────────┐   ┌────────────┐   ┌──────────────────┐   ┌──────────┐
   Command  ─────▶  │ Gateway  │─▶ │ Sequencer  │─▶ │ Matching Engine  │─▶ │  Output  │ ─────▶ Vec<Event>
(New/Cancel/Modify) └──────────┘   └────────────┘   └──────────────────┘   └──────────┘
                    validation and  assignment of     order book +          events:
                    normalization   seq and order_id  price-time matching    Trade / Resting / ...
```

The stages are connected by the [`Clob`](../../src/clob.rs) type. The public API is minimal:

```rust
let mut clob = Clob::new();
let events: Vec<Event> = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 5)));
```

## Principles

1. **Determinism.** A single instrument is processed by a single thread in strict order
   of sequence numbers. Same input → same output, always. This gives
   reproducibility, audit, replicas and (eventually) recovery from a journal.
2. **Integer math.** Prices are in ticks, quantities are in lots (`u64`).
   No `f64`: money calculations must be exact and repeatable.
3. **Events out, not mutations in.** The engine does not expose mutable
   state to the outside — it returns a stream of events, natural for market data and a journal.
4. **No parallelism inside a single book** — threads inside a book would break determinism.

## Pipeline stages

### Gateway — [src/gateway.rs](../../src/gateway.rs)

The first barrier. It checks the command for correctness before it reaches the engine:

- `qty == 0` → `RejectReason::ZeroQuantity`;
- zero price → `RejectReason::InvalidPrice`: a limit order with `price == 0`, a stop with
  `trigger == 0`, a stop-limit with `trigger == 0` or `price == 0`;
- `Modify` with `qty == 0` or `price == 0` is rejected by the same rules;
- a cancel always passes (the engine checks whether the order exists).

**Pre-trade risk (opt-in).** When a `RiskConfig` is configured ([src/risk.rs](../../src/risk.rs);
wired in via `Clob::with_risk` / `PersistentClob::open_with_risk`, default off, behavior
unchanged) the Gateway additionally checks:

- **tick/lot** — divisibility of the price by `tick_size` (limit price, `trigger`, both stop-limit fields, iceberg
  price) and of the volume/`display` by `lot_size`;
- **price band** — the price within a window of `±band_ticks` ticks around `mid = (best_bid + best_ask) / 2`
  (only Limit/Iceberg; stops and market skip it; on a cold/one-sided book, skipped). Mid
  is computed by `Clob` from the book and placed in the validation context — the Gateway does not read the book itself (stage
  separation preserved);
- **position limit** — net-signed worst-case `net + side's open volume + qty ≤ limit`
  (only `owner != 0`; anonymous is exempt).

The rejection reasons are `TickSize`, `LotSize`, `PriceBand`, `PositionLimit`. The config is static and immutable,
so determinism is preserved; the per-account net position and open volume are maintained by the book (see below).

### Sequencer — [src/sequencer.rs](../../src/sequencer.rs)

Assigns each event a monotonic `seq`, and each new order a unique
`order_id` (starting from 1). This is the heart of determinism: the order is fixed here and is not
changed afterwards. `seq` is also used as a logical timestamp.

### Matching Engine — [src/engine.rs](../../src/engine.rs)

Applies the validated and numbered command to the order book and produces events.
It is responsible for the semantics of order types and time-in-force:

- **FOK** — a `available_qty` precheck; if it cannot be fully executed, the order is
  rejected (`InsufficientLiquidity`) and does not enter the book.
- **Post-only** — a `would_cross` precheck; if the limit would immediately cross the spread,
  the order is rejected (`WouldCross`) before `Accepted` and does not enter the book (`Market`
  is always considered crossing). Otherwise it behaves like `Gtc` — it rests in the book.
- Otherwise — `Accepted`, then matching against the opposite side (`Trade` events).
- After matching:
  - remainder `0` → `Filled`;
  - remainder `>0` and it is `Limit` / `Iceberg` + (`Gtc` / `PostOnly`) → rests in the book
    (`Resting`); for `Iceberg` only the visible peak is shown in the book, the rest goes
    into the hidden reserve;
  - otherwise (`Market`, `Ioc`, an `Fok` that did not pass) → the remainder is canceled (`Canceled`).

Cancel: `execute_cancel` removes the order from the book (`Canceled`) or rejects
a non-existent one (`UnknownOrder`).

Modify (amend): `execute_modify` finds the resting order (otherwise `UnknownOrder`) and
emits `Modified`. Reducing the quantity at the same price is an in-place edit in `O(1)`
with priority preserved (then `Resting` with the new volume). A price change or an increase
in quantity is a loss of priority: the order is removed and, with the same `order_id`, re-runs
matching (on crossing the spread — `Trade`, then `Filled` or `Resting`). This way the book
stays uncrossed after an amend.

Stop orders (`Stop` / `StopLimit`) do not enter the main book immediately: the engine "parks"
them in a separate stop book ([src/stops.rs](../../src/stops.rs)) and emits `Accepted`. The trigger
is tied to the **last trade price**: a buy-stop is activated when `last >= trigger`, a sell-stop —
when `last <= trigger`. After any command that produced a trade (and also right after accepting
a stop), the engine runs the stops that fired: for each — `Triggered`, then ordinary matching
(`Stop` — as a market order, `StopLimit` — as a limit with the same TIF) through the same `settle`/`precheck` path
as `New`. The trades of the activated order move the last trade price, so activation is
**cascading**: one stop can pull the next. The cascade is deterministic (activation order —
by acceptance time) and finite (the set of dormant stops strictly decreases). Activation events are
tagged with the `seq` of the current command, the order is identified by `order_id`. `Cancel` removes a dormant
stop; `Modify` of a dormant stop is currently rejected (`UnknownOrder`).

Iceberg orders (`Iceberg { display }`) rest in the book as an ordinary limit, but show
only the visible part (peak, `display`); the remaining volume sits in the hidden reserve (a side
index in the book) and is not visible in `depth()` / `len()` / `available_qty()`. When the visible peak
is fully matched, a new peak `min(display, hidden)` is replenished from the reserve and placed **at
the tail** of its price level — behind all already-displayed orders, that is, with loss of
time priority (the standard iceberg rule). A sufficiently large opposing order
passes the whole iceberg in a single `match_against` call, layer by layer. Replenishment changes the book state,
but produces no events. As a taker the iceberg matches on the full `qty` (like a limit); `Fok`
sees only the displayed volume (the hidden reserve does not enter the liquidity check). `Modify`
of an iceberg treats `qty` as the new full volume, keeps `display` and re-places the order
(losing priority).

**Self-trade prevention (STP).** An order can carry an owner (`owner: AccountId`) and a mode
`stp: StpMode` (`Off` | `CancelTaker` | `CancelMaker` | `CancelBoth`; by default `Off`, `owner = 0`).
During matching the engine checks for a self-match **on each resting maker**: if `taker.stp != Off`,
`taker.owner != 0` and the maker's owner matches the aggressor's owner, instead of a trade the
**aggressor's** policy is applied (taker governs — the resting maker's `stp` is not read, it is enough for it to match by
owner): `CancelTaker` aborts the walk and removes the aggressor's remainder (`Canceled`), the maker is intact;
`CancelMaker` removes the encountered resting order (`Canceled`) and the aggressor continues walking the queue;
`CancelBoth` removes both. The decision is targeted — foreign liquidity on the path matches normally, so STP
does not disturb price-time priority for other participants. `owner` is an **opaque equality token**: the core
only compares it, stores it inline on `RestingOrder` and does not expose it in the public
market data projections. A fired stop carries its own `owner` / `stp` and on activation obeys
STP like an ordinary aggressor. STP is checked on `New`; `Modify` (amend) re-matches without STP.

**Fees (maker/taker).** When a `FeeConfig` is configured ([src/fees.rs](../../src/fees.rs);
builders `with_maker_ppm` / `with_taker_ppm`; default off) the engine computes the fee on **each
trade** at the point of its birth — the `on_trade` callback inside `settle`, where both
participants (the aggressor and the encountered maker) are in scope. The rates are given in **ppm of the notional** and are **signed**
(`i64`): `fee = price · qty · rate_ppm / 1_000_000` (intermediate `i128` with saturation, rounding
**by truncation toward zero**); a positive rate is a fee in favor of the exchange, a **negative maker
rate is a rebate** (a negative `maker_fee`). The result is exposed via two fields of `Event::Trade`
(`taker_fee` / `maker_fee`); the entire executed volume is charged, including the iceberg's hidden reserve
(a print on each matched layer). By the principle "events out, not state inside" the core **does not
accumulate** fees: the snapshot does not change, the journal is not
touched, `FeeConfig` is not persisted (like `RiskConfig` — the application sets it on open).
Wired in via `Clob::with_fees` / `with_risk_and_fees` and `PersistentClob::open_with_fees` /
`open_with_risk_and_fees`.

### Output — [src/output.rs](../../src/output.rs)

A single enumeration of events `Event`:

| Event | When |
| --- | --- |
| `Accepted` | the order was accepted by the engine |
| `Trade` | a trade took place (taker ↔ maker, price, volume; `taker_fee` / `maker_fee` — fees, signed `i64`) |
| `Resting` | the remainder rested in the book |
| `Filled` | the order was fully executed |
| `Canceled` | the order/remainder was removed |
| `Modified` | the order was modified (amend); the outcome events follow |
| `Triggered` | the stop order fired; the outcome events follow |
| `Rejected` | rejection (gateway, insufficient liquidity for FOK or crossing the spread for post-only) |

## Order book structure — [src/book/](../../src/book/)

The module is the `src/book/` directory: `mod.rs` (the book structure and operations, indexes), `matching.rs`
(`match_against` / `detach` — the matching algorithm and self-trade prevention), `accounts.rs`
(`AccountBook` — per-account aggregates for pre-trade risk). The submodules are children of `book`, so they
see the private fields of `OrderBook`; the split is dictated by the 400-line-per-file limit.

```
                 OrderBook
   ┌──────────────────────────────────────┐
   │ bids:  BTreeMap<Price, PriceLevel>    │   maximum price = best bid
   │ asks:  BTreeMap<Price, PriceLevel>    │   minimum price = best ask
   │ slab:  Vec<Node> + free list           │   arena of order nodes
   │ index: HashMap<OrderId, {side, slot}>  │   lookup of the node for cancel in O(1)
   │ reserves: HashMap<OrderId,{disp,hid}>  │   hidden parts of iceberg orders
   │ accounts: HashMap<AccountId,{net,open}>│   net position and open volume per account
   └──────────────────────────────────────┘
                    │
                    ▼  PriceLevel (one price level)
   ┌────────────────────────────────────┐
   │ head, tail: Option<u32>             │   intrusive doubly-linked FIFO list
   │ total_qty: Qty                      │   total volume of the level
   └────────────────────────────────────┘

   Node (in the slab): { order: RestingOrder, prev, next: Option<u32> }
```

- `bids` and `asks` are ordered by price (`BTreeMap`): the best price is at the end of the tree.
- Within a level the orders are linked by an intrusive doubly-linked list (`head`/`tail` →
  slots in the slab): arrival order is exactly the time priority. Insertion
  at the tail, matching from the head.
- The order nodes live in a shared arena (`slab`) with a free-slot list —
  allocations are reused, with no memory allocation per order in the steady state.
  The slab, the nodes and `PriceLevel` (the intrusive list and its operations `link_back` / `unlink`)
  are moved out into [src/slab.rs](../../src/slab.rs). `RestingOrder` additionally carries `owner`
  (for STP) — it does not enter the depth and the public projections.
- `index` maps `OrderId → (side, slot)`, so a cancel detaches the node
  from the list in `O(1)`, without walking the level.
- `reserves` holds the hidden parts of iceberg orders (`OrderId → {display, hidden}`):
  the visible peak is an ordinary node in the slab and in the level's `total_qty`, while the reserve sits separately
  and does not enter the depth. The entry appears only for icebergs with a non-empty reserve.
- `accounts` ([src/book/accounts.rs](../../src/book/accounts.rs)) maintains the per-account aggregates for
  pre-trade risk: the net position (`i128`, signed) and the open volume by side (the full remainder,
  incl. the iceberg's hidden reserve). It is updated on `insert` / `cancel` / `reduce` / matching /
  STP-`detach` — the book is the sole mutator of resting volume and sees both participants of each
  trade, so the aggregates do not drift from the book without threading `owner` through anonymized
  events; `owner == 0` (anonymous) is not counted. The net survives a snapshot, the open volume
  is reconstructed by re-loading the resting orders.

### Complexity

| Operation | Complexity | Note |
| --- | --- | --- |
| Best bid/ask | `O(log L)` | `L` — number of price levels |
| Insertion of remainder | `O(log L)` | lookup/creation of a level |
| Matching step | `O(log L)` per level + `O(1)` per order | FIFO from the head of the intrusive list |
| Cancel | `O(log L)` | lookup of the level in the tree + detaching the node from the list in `O(1)` |

## Matching algorithm

Price-time priority: the most aggressive price is executed first, at equal price —
the order that arrived earlier. The core is `OrderBook::match_against`:

```
match_against(taker_side, limit_price, qty):
    opp = asks if Buy else bids
    while qty > 0:
        best = min(asks) if Buy else max(bids)      # best opposite price
        if best is absent: break                     # liquidity ran out
        if not crosses(limit_price, best): break     # price no longer crosses
        level = opp[best]
        while qty > 0 and level is not empty:
            head = slab[level.head]                  # earliest maker
            if taker.stp != Off and head.owner == taker.owner:  # STP: self-match
                apply taker's policy (cancel taker → break; maker → remove and continue; both)
            traded = min(qty, head.qty)
            head.qty -= traded;  qty -= traded
            emit Trade(maker=head.id, price=best, qty=traded)
            if head.qty == 0:                        # node exhausted:
                index.remove(head.id)                #   remove from the index,
                level.head = head.next; free(slot)   #   detach the head, return the slot
        if level is empty: remove level best
    return qty                                       # unexecuted remainder
```

For an iceberg order the step `if head.qty == 0` does not remove the node, but replenishes the visible peak from
the hidden reserve (`min(display, hidden)`) and moves the node **to the tail** of the level: the hidden
part rejoins the queue, behind the already-displayed orders. The node is removed only when the
reserve is exhausted. The reserve strictly decreases, so replenishment is finite.

The crossing condition `crosses`:

- a market order (`limit_price = None`) — always crosses;
- `Buy` — `limit_price >= best_ask`;
- `Sell` — `limit_price <= best_bid`.

The same predicate is moved out into `OrderBook::would_cross` and is used by the post-only gate in the
engine: a maker-only order is rejected before acceptance if it would immediately match.

A trade is executed at the maker's price (`best`) — the one resting in the book, by the
price-time rule. A fully executed maker is immediately removed from `index`, and its slot
is returned to the slab's free-slot list.

Invariant: after processing any command the book is **not crossed** — an opposing order
that would cross the spread would necessarily have matched on arrival (checked by the
test `book_is_never_crossed_after_matching`).

## End-to-end example

Book: ask `101 ×10`, ask `102 ×5`, bid `100 ×7`.
An aggressive `Buy` limit `101 ×12` (`Gtc`) arrives:

1. Gateway: ok. Sequencer: `seq=4`, `order_id=4`. → `Accepted`.
2. Matching against asks: the best ask `101` crosses (`101 >= 101`) → trade `101 ×10`.
   The next ask `102` no longer crosses (`101 < 102`) → stop. Remainder `2`.
3. `Limit` + `Gtc`, remainder `>0` → rests in the book as bid `101 ×2`. → `Resting`.

Resulting book: bid `101 ×2`, bid `100 ×7`, ask `102 ×5`, spread `1`.
(This is exactly the scenario printed by `examples/basic.rs`.)

## Market data — snapshots, incremental updates and the trade tape — [src/marketdata.rs](../../src/marketdata.rs)

Market data is a **read projection** of the book for external observers, separate from the private
event stream: the engine writes the book, market data only reads it. It is implemented as two methods
on `Clob` over the public `book()` / `current_seq()`, without changes to the deterministic core:

- `l2_snapshot(depth)` — **market-by-price**: aggregated depth by price levels
  (`L2Snapshot` with `bids` / `asks` of `L2Level { price, qty }`), the best price first, no more than
  `depth` levels per side. The source is `OrderBook::depth`.
- `l3_snapshot()` — **market-by-order**: the book by individual orders (`L3Snapshot` of
  `L3Order { id, side, price, qty }`), best-first by price and FIFO within a level — that is, the queue
  position is visible. The source is `OrderBook::resting_orders` (a stable sort by price
  preserves the queue order within a level).

Both snapshots are tagged with the current `seq`, so that the consumer can order them relative to future
incremental updates (the next step of v0.4). The projection is **public and anonymized**: for
iceberg orders only the visible peak is given out — the hidden reserve does not enter L2/L3 (privacy
is defined by what the participant *placed* in the book, not by the granularity of the feed). L2 is the aggregation of
L3 by price: `sum(L3 qty per level) == L2 qty` (the invariant is checked by the test
`l2_equals_l3_aggregated`).

### Incremental L2 updates — `L2Update` / `L2Feed`

A snapshot is the whole book; to avoid resending it on every change, market data can give out
**incremental L2 deltas**. `L2Feed` is a stateful read projection: `apply(&events, book)` turns
the `Vec<Event>` of one command into an `L2Update { seq, bids, asks }` — a list of **only the changed** price
levels (a level with `qty == 0` means deletion; prices — best-first on each side). The frame carries
the command's `seq`, a shared anchor with the snapshot: the consumer takes `l2_snapshot` at `seq = N` (or seeds the feed
with the same state via `L2Feed::from_book`) and applies the subsequent deltas, keeping the local book
in sync without resending the whole depth.

Which levels are affected the feed derives **from events**, without scanning the book: for `Trade` — the price on the maker's
side (`taker_side.opposite()`); for `Resting` — the side and price of the remainder that rested; for `Canceled` /
`Filled` / `Modified` — the order's former position from the internal mirror `order_id → (side, price)`
(these events carry only `order_id`). The new aggregate of each affected level is **read from the book**
(`OrderBook::level_qty`), not accumulated from events, so the delta is exact even in two cases
invisible from the event stream: **iceberg replenishment** (there is no event, but the replenishment always happens at the trade
price, and that is already in the set of affected levels) and **modify-reprice** (the old level is added on `Modified`
before the subsequent `Resting` updates the mirror to the new price). The projection stays anonymized —
the iceberg's hidden reserve is not in the book, and it does not enter the deltas. Determinism is preserved: the set of
affected levels is a `BTreeSet`, the output is explicitly sorted, `HashMap` iteration does not affect the output.

The feed is co-located with the engine (it reads the book).

### Incremental L3 updates (market-by-order) — `L3Update` / `L3Feed` — [src/l3feed.rs](../../src/l3feed.rs)

An L2 delta aggregates a level into a single number; an L3 delta works at the granularity of the **individual order** and
carries the **queue position**. `L3Feed::apply(&events, book)` gives out `L3Update { seq, deltas }`, where
`L3Delta` is `Added { id, side, price, qty }` (the order rests at the **tail** of its level),
`Reduced { id, qty }` (the new volume, the position preserved) or `Removed { id }` (the order left the book).
The consumer holds an ordered list of orders per level (plus `id → level`) and applies the deltas,
exactly reproducing the book's FIFO.

Which levels are affected the feed derives **from events** — by the same logic as `L2Feed` (the maker's side for
`Trade`; the mirror `id → (side, price)` for `Resting` / `Modified` / `Canceled` / `Filled`). But the new
queue order of each affected level is **reconciled with the book** via `OrderBook::level_orders(side,
price)` (the level's orders head→tail), not accumulated from events. The deltas are the diff of the old level list
against the new one: the common head subsequence (orders that kept their position) gives `Reduced` on a change of
volume, those that dropped out of it — `Removed`, the tail remainder of the new list — `Added`. This way the two cases "silent" by
events are expressed correctly: **iceberg replenishment** (the visible peak is exhausted → the new peak rests at the
tail of the level) and **loss of priority on amend** (a price change or a volume increase) — both as
remove-then-add. Therefore in a frame the deltas are ordered `Removed` → `Reduced` → `Added`: by the time of the insertion at the
tail all departures and reorderings have already been applied, and the queue order at the consumer matches the book
bit-for-bit. The iceberg's hidden reserve does not enter the deltas (only the visible peak is given out).

`L3Feed::new()` starts from an empty book, `L3Feed::from_book(book)` — seeding from an `l3_snapshot` snapshot with the same
`seq`. Determinism is preserved: the affected levels are a `BTreeSet` of prices by side, `level_orders` gives out
FIFO order, the iteration of the internal `HashMap`s (`id → level`, the mirror of levels) does not affect the output.

### End-to-end sequencing of data channels

All market data channels are anchored to the command's `seq` through a single `command_seq(&events)` (one source
of truth in [src/marketdata.rs](../../src/marketdata.rs)): the events of one command carry the shared `seq`
of the sequencer, and each channel tags its frame with it. The snapshots `l2_snapshot` / `l3_snapshot` carry
`Clob::current_seq()` (the value after the command), the increments `L2Update` / `L3Update` and the prints `TapeTrade` —
the same command `seq`. For one command all channels carry **one** `seq`, so the consumer: (1) takes a
snapshot at `seq = N`, (2) applies the increments with `seq > N`, (3) reconciles the book channels with the trade tape by
the shared anchor. This is a contract between the channels, not a coincidence: `current_seq()` after the command equals the `seq` of its
events, and therefore the `seq` of all its frames.

### Trade tape — `TradeTape` / `TapeTrade` — [src/tape.rs](../../src/tape.rs)

Snapshots and L2 deltas describe the **book state**; the trade tape is a separate channel of the **stream
of executions**. `TradeTape::apply(&events)` extracts from the events of one command one anonymized
print `TapeTrade { seq, price, qty, taker_side }` per `Event::Trade` (in execution order —
on a sweep of several levels best price first), returns the command's prints and retains them in a
bounded ring history (`new()` — without a limit, `bounded(cap)` — the last `cap`; reading —
`recent()` from old to new, `last()`, `len()`). Each print carries the command's `seq` — the same anchor as
the snapshots and `L2Update`, so the channels are ordered with one another by `seq`.

Unlike the L2/L3 projections the tape is **book-free**: `apply` takes only `&[Event]` and does not read the book —
the tape can be maintained by a consumer that has only the event stream (something the co-located L2 feed cannot yet
do). Anonymity here is about the owner and identifiers: order ids do not enter the print. But the
executed volume is a public fact and is printed **in full**: a pass over an iceberg gives a print on each
matched layer, including the volume from the hidden reserve. This is a deliberate asymmetry with the book projections —
the book hides the *resting* reserve (what the participant placed), while the tape prints the *executed* (what happened on the
market). The tape does not enter the state snapshots (trades are a stream, not state): on a journal replay the
commands are re-executed and the `Trade` events arise anew, so the consumer reconstructs the same
tape from the replayed stream. Determinism is preserved — the tape only projects an already-deterministic stream
of events.

The tape is the last channel of v0.4: with incremental L3 (market-by-order) and end-to-end sequencing of channels the
Market data stage is closed.

## Where parallelism lives

Inside a single book there is no parallelism — it would break determinism. `Clob` processes the book strictly in a single thread.

## Persistence, snapshots and replay — [src/persist.rs](../../src/persist.rs)

A superstructure over the deterministic core (v0.3). `PersistentClob` wraps `Clob`
and maintains a **write-ahead log** of input commands: a command is serialized and flushed to disk
(`fsync`) **before** the events return to the caller — a confirmed order survives a
process crash. On open the state is reconstructed by **replay**: the recorded stream of
commands is re-submitted into a fresh `Clob`. Since the engine is deterministic, the book, `order_id` and
`seq` are reproduced exactly (commands rejected by the gateway are also journaled — they consume
`seq`, otherwise the numbering on replay would diverge).

The journal format is a custom binary one, `std`-only (without serde and external crates):

```
segment = [ magic "CLBW" (4) | version u16 | base_seq u64 ]   header once
          then blocks:
          [ block_len u32 | rec_count varint | records… | crc32 u32 ]   one block = one fsync
record  = [ tag u8 | (for New) flags side|type|tif | varint fields by type ]
```

The numeric fields are LEB128-varint ([src/codec.rs](../../src/codec.rs)); the same codec is reused by
snapshots. The encoding/decoding of `Command` and the header are in
[src/journal.rs](../../src/journal.rs); writing with group commit (`Journal`) and reading
(`read_commands` / `read_segment`) are in [src/wal.rs](../../src/wal.rs). On recovery the reader
checks the `magic`/version and the CRC of each block; the last record interrupted mid-write (CRC does not
match or the block is incomplete) is cut off — the journal is recovered up to the last whole record.

### Snapshots (checkpoint) — [src/snapshot.rs](../../src/snapshot.rs)

So that recovery does not replay the whole journal from scratch, `checkpoint()` saves a **snapshot**
of the full deterministic state and then **rotates** the journal, leaving only the tail:

1. `journal.commit()` — flush the accumulated block.
2. Write the snapshot atomically: to a temporary file `<journal>.snap.tmp` → `fsync` → `rename` to
   `<journal>.snap`. This is the checkpoint commit point.
3. `journal.rotate(N)` — replace the journal with a fresh segment with `base_seq = N` (`N` — the current `seq`),
   also via a temporary file + `rename`. The old commands (`seq ≤ N`) are now only in the snapshot.

The snapshot serializes everything that affects further processing: the sequencer counters `seq` and
`next_order_id` (the latter is not derived from `seq` — `Cancel`/`Modify` spend `seq`, but not
`order_id`), the last trade price (needed for stop triggers), all resting orders in priority
order together with the icebergs' hidden reserves, the dormant stops and the accounts' net positions (for
position limits; the accounts are sorted by `AccountId`). The format is `magic "CLBS"` + version (`3`),
varint fields, a single frame with CRC32 at the end. `RiskConfig` is not written into the snapshot — the application sets
it on `open_with_risk` (on replay the same config is needed); the open volume per account is not stored —
it is reconstructed by re-loading the resting orders.

Recovery with a snapshot (`PersistentClob::open`):

```
open(journal, snapshot):
    (clob, applied) = snapshot exists ? (restore(snapshot), snapshot.seq) : (Clob::new(), 0)
    (base_seq, commands) = read_segment(journal)
    for the i-th command:  seq = base_seq + i + 1
        if seq > applied:  clob.submit_into(command)   # we replay only the tail
    journal = Journal::open_base(journal, applied)      # an empty/new segment gets base = applied
```

**Reconciliation is idempotent by `seq`** — this is exactly the crash safety of the checkpoint. Only
commands with `seq` greater than what is covered by the snapshot are applied, so it does not matter whether `rotate` managed to run:
if the checkpoint was interrupted after writing the snapshot but before the rotation, the journal still contains commands
`seq ≤ N` — they are simply skipped (not applied again). The commands `seq > N` always lie
in the journal and are replayed. A corrupt snapshot (CRC does not match) is rejected as `CorruptSnapshot`:
then the journal is the sole source of truth, and silently substituting the state is not allowed.

The determinism invariant is not broken: the persistence layer does I/O, but introduces into the core
neither a clock, nor threads, nor `HashMap`-dependent iteration (the snapshot walks the orders by price
levels and FIFO within a level — a deterministic order); the framing, the commit and the checkpoint are
driven by a record counter and an explicit application call, not by a wall clock.

## Module map

| Module | Contents |
| --- | --- |
| [src/types.rs](../../src/types.rs) | primitives: `OrderId`, `AccountId`, `Price`, `Qty`, `Side`, `OrderType`, `TimeInForce`, `StpMode` |
| [src/order.rs](../../src/order.rs) | input API: `Command`, `NewOrder` (incl. `owner` / `stp`), `CancelOrder`, `ModifyOrder` |
| [src/gateway.rs](../../src/gateway.rs) | the Gateway stage; pre-trade risk over `RiskConfig` |
| [src/risk.rs](../../src/risk.rs) | `RiskConfig` — pre-trade risk parameters (tick/lot, price band, position limit) |
| [src/fees.rs](../../src/fees.rs) | `FeeConfig` — maker/taker fee rates (ppm of the notional, signed) and the trade's fee computation |
| [src/sequencer.rs](../../src/sequencer.rs) | the Sequencer stage |
| [src/book/mod.rs](../../src/book/mod.rs) | order book: levels, slab index, reserves, accounts index |
| [src/book/matching.rs](../../src/book/matching.rs) | the matching algorithm and self-trade prevention (`match_against` / `detach`) |
| [src/book/accounts.rs](../../src/book/accounts.rs) | `AccountBook` — net position (`i128`) and open volume per account |
| [src/slab.rs](../../src/slab.rs) | the slab arena of nodes and the level's intrusive FIFO list (`Slab` / `PriceLevel`) |
| [src/engine.rs](../../src/engine.rs) | the Matching Engine stage |
| [src/stops.rs](../../src/stops.rs) | the stop-order book (triggers, cascade activation) |
| [src/output.rs](../../src/output.rs) | the event model |
| [src/marketdata.rs](../../src/marketdata.rs) | market data projections: L2 / L3 book snapshots, incremental L2 deltas (`L2Feed`), the channels' shared anchor (`command_seq`) |
| [src/l3feed.rs](../../src/l3feed.rs) | incremental market-by-order: `L3Feed` / `L3Update` / `L3Delta` — per-order deltas with queue position |
| [src/tape.rs](../../src/tape.rs) | the trade tape: `TradeTape` / `TapeTrade` — a book-free stream of prints over events |
| [src/error.rs](../../src/error.rs) | rejection reasons |
| [src/clob.rs](../../src/clob.rs) | pipeline wiring, the public `Clob` |
| [src/codec.rs](../../src/codec.rs) | low-level codec: varint/LE primitives, CRC32 |
| [src/journal.rs](../../src/journal.rs) | journal format: header (magic+version), serialization of `Command` |
| [src/wal.rs](../../src/wal.rs) | WAL storage: `Journal` (write + `fsync` + rotation), `read_segment` / `read_commands` (reading) |
| [src/snapshot.rs](../../src/snapshot.rs) | state snapshot format (`magic "CLBS"`): capture/restore of `Clob`, atomic write |
| [src/persist.rs](../../src/persist.rs) | `PersistentClob` — a wrapper over `Clob` with a journal, snapshots and replay |
