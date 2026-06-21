# Changelog

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Maker/taker fees (v0.5, phase 3).** New module `src/fees.rs` and public type `FeeConfig`
  with builders `with_maker_ppm(rate)` / `with_taker_ppm(rate)`. Rates are given in **ppm** (parts per
  million of the **notional** `price·qty`) and are **signed** (`i64`): a positive rate is a fee in favor of
  the exchange, a **negative maker rate is a rebate** (a payout to the maker). It is wired in via
  `Clob::with_fees(cfg)` / `Clob::with_risk_and_fees(risk, cfg)` and
  `PersistentClob::open_with_fees(path, cfg)` / `open_with_risk_and_fees(path, risk, cfg)`;
  `Clob::new()` / `default()` and the previous constructors remain **without fees** (default off, rates `0`),
  so existing behavior does not change. The fee is computed **on each trade** at the point of its birth
  (the engine, the `on_trade` callback, where both participants are visible) and is output via two new `Event::Trade` fields —
  `taker_fee: i64` and `maker_fee: i64`. Formula: `fee = price·qty·rate_ppm / 1_000_000`, computed intermediately in
  `i128` with saturation, **rounding by truncation toward zero** (symmetric for fee and rebate). The full
  executed volume is charged, including the iceberg's hidden reserve (a print on each matched layer).
  By the "events out, not state in" decision the fee core **does not accumulate**. Therefore the **snapshot does not change** (`CLBS` stays at version `3`), the journal
  (`CLBW`) is untouched, and `FeeConfig`, like `RiskConfig`, is **not persisted** — the application sets it at
  open time (replay needs the **same** config; that said, fees do not affect the book's state, only the
  numbers that are output). Fees **do not leak** into the anonymized market data channels — `taker_fee` / `maker_fee`
  do not make it into L2/L3 snapshots, increments, or the trade tape (`TapeTrade`). New public type — `FeeConfig`.
  The deterministic core is not broken, and there are no new dependencies (only `std`). **Closes the v0.5 milestone.**
- **Pre-trade risk in the Gateway (v0.5, phase 2).** New module `src/risk.rs` and public type
  `RiskConfig` with builders `with_tick(tick_size)` / `with_lot(lot_size)` / `with_price_band(band_ticks)` /
  `with_position_limit(limit)`. It is wired in via `Clob::with_risk(cfg)` and `PersistentClob::open_with_risk(path, cfg)`;
  `Clob::new()` / `default()` and `PersistentClob::open` remain **without checks**, so existing
  behavior does not change (risk control is strictly opt-in, default off). The Gateway became stateful (it carries
  `RiskConfig`), but the public `Gateway::validate(&Command)` still does only the structural check;
  the risk path is internal. Checks:
  - **Tick/lot.** The price is a multiple of `tick_size` (limit price, the stop's `trigger`, both stop-limit fields, the iceberg price),
    the volume and the iceberg's `display` are multiples of `lot_size`. Values of `0`/`1` disable the corresponding check.
    Market skips the price checks (there is no price), but the lot applies to `qty`.
  - **Price band.** An order is rejected if the price is farther than `band_ticks` ticks from **mid = (best_bid + best_ask) / 2**:
    the window `[mid − band_ticks·tick, mid + band_ticks·tick]` (tick=`max(tick_size, 1)`). The dynamic reference is taken
    from the book (`OrderBook::mid()`), but the Gateway does not read it itself — `Clob` passes mid into the validation
    context (stage separation is preserved). On a **cold/one-sided book** (mid is undefined) the band is skipped.
    The band applies only to Limit/Iceberg; stops (which sit outside the market) and market — pass through.
  - **Position limits (worst-case, net-signed).** For `owner != 0`, `|net| ≤ limit` is checked in the worst case:
    for a buy `net + open_buy_volume + qty ≤ limit`, for a sell `−net + open_sell_volume + qty ≤ limit`.
    An anonymous order (`owner == 0`) is exempt. On `Modify` the order's own current contribution to the open volume is subtracted,
    then the new `qty` is added. The open volume accounts for the iceberg's full remainder (visible + hidden reserve).
  Position accounting is done by the **book itself** (a single source of truth): `match_against` sees both participants of each trade,
  so the net position (`i128`) and the open volume per side are updated inside the book on `insert` / `cancel` /
  `reduce` / matching / STP `detach` — without passing `owner` through anonymized events. `Clob`/the engine only
  **read** the aggregates for validation. The net position **survives** snapshot and replay: the snapshot is versioned
  (`CLBS` → version `3`, a net-positions section, accounts sorted by `AccountId`); the journal (`CLBW`) does not change —
  the commands are the same, and net is reconstructed by re-laying the resting orders and replaying the trades. `RiskConfig` is
  **not written** to the journal/snapshot — the application sets it at open time; on replay you must supply the **same** config, otherwise history diverges.
  New public types — `RiskConfig`; new `RejectReason`s — `TickSize`, `LotSize`, `PriceBand`, `PositionLimit`.
  Refactoring: `src/book.rs` (which was at the 400-line limit) was split into a directory-module `src/book/` —
  `mod.rs` (book + indexes + accounts index), `matching.rs` (`match_against` / `detach`), `accounts.rs` (`AccountBook`:
  net + open volume). The deterministic core is not broken (`i128` net and `u64` volume, `HashMap` iteration does not affect
  output — the snapshot is sorted by account), and there are no new dependencies (only `std`). Known boundaries: dormant stops are not
  accounted for in the open volume until they fire; `RiskConfig` on replay is set by the application. Closes the second item of v0.5;
  maker/taker **fees** remain.
- **Order account/owner and self-trade prevention (STP) (v0.5, phase 1).** An order gained
  optional fields `owner: AccountId` (a new `u64` primitive in `src/types.rs`; `0` is anonymous /
  not set) and `stp: StpMode` (`Off` | `CancelTaker` | `CancelMaker` | `CancelBoth`), set by
  builders `NewOrder::with_owner(id)` / `with_stp(mode)`; the default constructors set
  `owner = 0`, `stp = Off`, so existing behavior does not change (STP is strictly opt-in).
  **Self-trade prevention** fires at the moment of matching, per each resting maker: on
  `taker.stp != Off && taker.owner != 0 && taker.owner == maker.owner`, instead of a trade the
  **aggressor's** policy is applied (taker governs) — the resting maker's `stp` is not read; it is enough for it to
  match by owner. `CancelTaker` cancels the aggressor's remainder (`Canceled`), the maker is intact;
  `CancelMaker` cancels the encountered resting order (`Canceled`) and the aggressor proceeds further down the queue;
  `CancelBoth` cancels both. The decision is made **per each** maker while walking the level, so
  someone else's liquidity matches normally, while a self-match is resolved precisely. STP applies also
  to a **fired stop** (it carries its own `owner` / `stp` and on activation acts as an aggressor).
  `owner` is stored inline on `RestingOrder` (cache-locally for the matching loop) and **survives**
  snapshot and replay: the journal and snapshot are versioned (`CLBW` / `CLBS` → version `2`); the `New`
  record carries `owner` as a variant and `stp` in the free bits of the flags byte. The market data projection stays
  **anonymized** — `owner` does not make it into L2 / L3 / the trade tape. New public types —
  `AccountId`, `StpMode`, `MatchOutcome`. The deterministic core is not broken (the new `u64`
  is compared only for equality, `HashMap` iteration does not affect output), and there are no new dependencies
  (only `std`). Known boundaries: STP is checked on `New` (not on `Modify` — amend re-matches
  without STP); the `Fok` / post-only precheck counts the opposite volume in full, without subtracting its own
  liquidity. Closes the first item of v0.5; pre-trade risk and fees are next.
- **Market data: incremental L3 updates and end-to-end channel sequencing (v0.4, phase 4 — closes v0.4).**
  New module `src/l3feed.rs` and public types `L3Delta` (`Added { id, side, price, qty }` /
  `Reduced { id, qty }` / `Removed { id }`), `L3Update { seq, deltas }` and `L3Feed` — an incremental
  **market-by-order** channel with queue position. `L3Feed::apply(&events, book)` turns the events
  of a single command into per-order deltas: the affected price levels are derived from the events (as in `L2Feed`),
  and the queue order within each level is reconciled against the book via the new read accessor
  `OrderBook::level_orders(side, price)`. Survivors (orders that kept their position) yield `Reduced`,
  the departed — `Removed`, new/moved-to-tail ones — `Added`; the deltas are emitted in the order
  remove → reduce → add, so the **iceberg replenishment** and the **loss of priority on amend** (moving to the
  tail of the level) are correctly expressed as remove-then-add and exactly reproduce the book's FIFO at the consumer.
  `L3Feed::new()` starts with an empty book, `L3Feed::from_book(book)` — a seed from a snapshot with the same `seq`.
  The projection stays **public and anonymized**: only the iceberg's visible peak is given out, the hidden reserve does not
  make it into the deltas.
  **End-to-end sequencing of the data channels.** All market data channels are anchored on the command's `seq` via a single
  `command_seq` (one source of truth instead of scattered output): the `l2_snapshot` / `l3_snapshot` snapshots
  carry `Clob::current_seq()`, the `L2Update` / `L3Update` increments and the `TapeTrade` prints — the same command
  `seq`. For a single command all channels carry **one** `seq`, so the consumer applies increments with
  a `seq` greater than the snapshot's and reconciles the channels with each other by the common anchor. New public types — `L3Delta`,
  `L3Feed`, `L3Update`; the read accessor `OrderBook::level_orders` was added. The deterministic core is not
  changed, and there are no new dependencies (only `std`). With this step **v0.4 (Market data) is closed**.
- **Market data: trade tape (v0.4, phase 3).** New module `src/tape.rs` and public
  types `TapeTrade` (an anonymized print: `seq` + `price` + `qty` + `taker_side`) and `TradeTape` —
  a separate data channel on top of the event stream. `TradeTape::apply(&events)` extracts one print
  per each `Event::Trade` (in execution order — when sweeping several levels, best price first),
  returns the command's prints and accumulates them in a bounded ring history: `new()` — no limit,
  `bounded(cap)` — only the last `cap`; reading — `recent()` (from old to new), `last()`, `len()`,
  `is_empty()`. Unlike the L2/L3 projections, the channel is **book-free**: `apply` takes only `&[Event]` and does not
  touch the book — the tape can be maintained by a consumer that sees only the event stream. Each print carries
  the command's `seq` — a common anchor with the book channel (`L2Update` and snapshots). The anonymity is about the owner and
  identifiers (order ids do not make it into the tape); the executed volume is public and is printed in full,
  so a pass over an iceberg yields a print on each matched layer, including the volume from the hidden reserve
  (the book hides the *resting* reserve, the tape prints the *executed*). The deterministic core is not changed,
  and there are no new dependencies (only `std`). New public types — `TapeTrade`, `TradeTape`. The remaining
  steps of v0.4 (the L3 increment and end-to-end channel sequencing) were closed by the next phase.
- **Market data: incremental L2 updates (v0.4, phase 2).** New public types
  `L2Update` (a sparse frame: `seq` + `bids` / `asks` of `L2Level`, a level with `qty == 0`
  means a removal, prices best-first on each side) and `L2Feed` — a stateful read projection
  that turns the `Vec<Event>` of a single command into deltas of **only the changed** price levels.
  `L2Feed::apply(&events, book)` derives the affected levels from the events (the maker's side on `Trade`;
  an internal mirror `order_id → (side, price)` for `Resting` / `Modified` / `Canceled` / `Filled`,
  which do not carry the price) and reconciles each level's new aggregate against the book — so the deltas are exact
  even where the event stream is "silent": on an **iceberg replenishment** (there is no event, but the replenishment is always at
  the trade price, and that is already among the affected ones) and on a **modify reprice** (the old level is added on
  `Modified` before `Resting` updates the mirror to the new price). `L2Feed::new()` starts with
  an empty book, `L2Feed::from_book(book)` — a seed from the current state (to pair with `l2_snapshot`
  with the same `seq`): the snapshot sets the base, the deltas keep it in sync without resending the whole
  book. The frame carries the command's `seq` — a common anchor with the snapshot. The projection stays **public and
  anonymized**: the iceberg's hidden reserve does not make it into the deltas. One read accessor was added,
  `OrderBook::level_qty(side, price)`; the deterministic core is not changed, and there are no new dependencies
  (only `std`). New public types — `L2Update`, `L2Feed`. The L3 increment, the trade tape, and
  the consistency of sequence numbers between channels — the remaining steps of v0.4.
- **Market data: L2 and L3 book snapshots (v0.4, phase 1).** Two new methods on `Clob`:
  `l2_snapshot(depth)` — **market-by-price**, depth aggregated by price levels
  (`L2Snapshot` with `bids` / `asks` of `L2Level { price, qty }`, the best price first, no more than
  `depth` levels per side), and `l3_snapshot()` — **market-by-order**, the book by individual
  orders (`L3Snapshot` of `L3Order { id, side, price, qty }`, best-first by price and FIFO within
  a level — queue position is visible). Both snapshots are tagged with the current `seq` (`current_seq`), so that
  the consumer can order them relative to future incremental updates. The projection is
  **public and anonymized**: for iceberg orders only the visible peak is given out — the hidden reserve
  **does not make it** into L2/L3. L2 is the aggregation of L3 by price (the invariant is checked by a test). Implemented
  as a read projection in the new module `src/marketdata.rs` on top of the public `book()` / `current_seq()`:
  the deterministic core is not changed, and there are no new dependencies (only `std`). New public types —
  `L2Level`, `L2Snapshot`, `L3Order`, `L3Snapshot`. Incremental updates, the trade tape, and
  the consistency of sequence numbers between channels — the next steps of v0.4.
- **Persistence: book snapshots (v0.3, phase 2).** New method
  `PersistentClob::checkpoint()` writes to disk a **snapshot** of the full deterministic
  state (the sequencer's `seq` and `next_order_id`, the last trade price, all resting orders
  in priority order together with the icebergs' hidden reserves, the dormant stop orders) and then
  truncates the journal to the tail (segment rotation with a new `base_seq`). Recovery
  (`PersistentClob::open`) now loads the snapshot and **replays only the tail** of the journal, rather
  than the whole stream from scratch. The snapshot format is a custom binary one, `std`-only: header
  `magic "CLBS"` + version, varint fields, a common frame with CRC32 (the same codec as the journal).
  The snapshot write is atomic (write to a temporary file → `fsync` → `rename`). Reconciliation of the
  snapshot and the journal is idempotent by `seq`: on recovery only the journal commands
  with a `seq` greater than what the snapshot covers are applied, so a checkpoint interrupted mid-way (the snapshot
  is written, the journal not yet rotated) leads neither to a double application nor to a loss of commands.
  A corrupt snapshot (CRC does not match) is rejected as `JournalError::CorruptSnapshot` — the journal
  meanwhile remains the source of truth. The snapshot is stored next to the journal (`<journal>.snap`),
  the public API is extended with one method, `checkpoint`; there are no new dependencies (only `std`),
  and the deterministic core is not changed. New error variant — `JournalError::CorruptSnapshot`.
- **Persistence: command journal and replay (v0.3, phase 1).** New wrapper
  `PersistentClob` wraps `Clob` and maintains a write-ahead journal of input commands: each
  command is serialized and flushed to disk (`fsync`) **before** the events are returned, so that
  an acknowledged order survives a process crash. On open (`PersistentClob::open`) the
  state is restored by **replay** — the recorded command stream is re-fed into a fresh
  `Clob`; thanks to determinism the book, `order_id` and `seq` are reproduced exactly (commands rejected
  by the gateway are also journaled — they consume a `seq`). The journal format is a custom
  binary one, `std`-only: segment header `magic "CLBW"` + format version, command records with
  varint fields, blocks are framed by length + CRC32 (group commit — `submit_batch`).
  A last record interrupted half-written is cut off by CRC on recovery.
  Low-level access: `Journal` (the writer), `read_commands` (the reader). New public
  types — `PersistentClob`, `Journal`, `JournalError`, `CodecError`, and the function `read_commands`.
  The deterministic core is not changed: persistence is a layer over `Clob`, without clocks or threads;
  there are no new dependencies (only `std`). Book snapshots to speed up recovery are the next
  step (phase 2).
- **Iceberg (`OrderType::Iceberg { display }`)** — an order with a visible part (peak,
  `display`) and a hidden reserve. Constructor `NewOrder::iceberg(side, price, qty, display)`,
  where `qty` is the full volume and `display` is the size of the visible part. In the book, `depth()`,
  `len()`, `total_qty` and `available_qty()` account for **only the visible peak**; the reserve
  is stored in a side index of the book and is not shown. When the visible peak fully
  matches, a new peak `min(display, hidden)` is replenished from the reserve and placed **at the tail**
  of its price level — losing time priority to the already displayed orders
  (the standard iceberg rule). A sufficiently large taker "sweeps" the whole iceberg
  (including the hidden part) in a single pass, layer by layer. The replenishment **does not produce events**
  (it is reflected in `depth()`); incremental updates are a separate item of v0.4. As a taker, the
  iceberg matches on the full `qty`, like an ordinary limit. The Gateway rejects `display == 0`
  (`ZeroQuantity`) and `price == 0` (`InvalidPrice`); `display >= qty` reduces to an ordinary
  fully visible order. For `Fok` the hidden reserve is **not accounted for** in the liquidity
  check (only the displayed volume is counted). A `Modify` of an iceberg treats `qty`
  as the new full volume, keeps `display` and always re-inserts the order anew (losing
  priority). No new `Event` / `RejectReason` was added.
- **Stop and stop-limit (`OrderType::Stop` / `OrderType::StopLimit`)** — deferred orders with a trigger price. Constructors `NewOrder::stop(side, trigger, qty)` (on firing behaves like a market order) and `NewOrder::stop_limit(side, trigger, limit, qty)` (on firing — a limit order at the price `limit`). The Gateway rejects a stop with `trigger == 0` and a stop-limit with `trigger == 0` or `price == 0` as `InvalidPrice`. Firing is tied to the **last trade price**: a buy-stop is activated when `last >= trigger`, a sell-stop — when `last <= trigger`. Until it fires the order "sleeps" in a separate stop book and is **not visible** in the book's `depth()` / `len()`; if the market is already past the trigger, the stop fires immediately on acceptance. Activation is **cascading**: the trades of a fired order move the price and may activate the next stops (the activation order is by acceptance time, deterministically). `Cancel` cancels a dormant stop; `Modify` of a dormant stop is not yet supported (it is rejected as `UnknownOrder`).
- **Event `Triggered`** — a stop order has fired and is now going through matching; next come the ordinary outcome events (`Trade` / `Resting` / `Filled` / `Canceled`), as for `New` after `Accepted`. Activation events are tagged with the current command's `seq`, the order identity — by `order_id`.
- **`Clob::pending_stops()`** — the number of dormant (not yet fired) stop orders.
- **Post-only (`TimeInForce::PostOnly`)** — a "maker-only" order: if the limit would immediately cross the spread, it is rejected (`RejectReason::WouldCross`) before `Accepted` and does not make it into the book; otherwise it behaves like `Gtc` and rests in the book. The check is in the engine, before acceptance (like the `Fok` precheck); for `Market` a crossing is always counted.
- **Command `Modify` (amend)** — a change of the price and/or quantity of a resting order (`Command::Modify`, `ModifyOrder::new`). A reduction of the quantity at the same price preserves time priority (an in-place edit in `O(1)`); a change of price or an increase of quantity loses priority — the order is canceled and re-matches anew with the same `order_id`, and on crossing the spread it executes.
- **Event `Modified`** — confirmation of acceptance of `Modify`; next come the outcome events (`Resting` / `Trade` / `Filled`), as for `New` after `Accepted`.

### Changed

- **Order book** — the slab arena with the intrusive FIFO list (`Node`, `Slab`, `PriceLevel` and the operations `link_back` / `unlink`) was extracted from `src/book.rs` into a new module `src/slab.rs` (a split by responsibility: `book.rs` stays under the 400-line limit). Behavior and complexity did not change.
- **Order book** — within a price level, an intrusive doubly-linked FIFO list instead of `VecDeque`; the order nodes are stored in a shared arena (slab) with a free-slot list. Cancellation within a level is now `O(1)` instead of `O(n)`.
- **Matching Engine** — the `Vec` allocation for the identifiers of executed orders was removed: the maker is removed from the index directly in the matching loop, and the node's slot is returned to the pool.
- **Matching Engine** — the common matching path (TIF prechecks, matching, resting the remainder/cancellation) was extracted into `settle` / `precheck` and is reused by new orders, amend and stop activation; the stop book and cascade activation by the last trade price were added. A `Modify` that produced a trade can now also activate stops.

For the rest of what is planned, see [ROADMAP.md](ROADMAP.md).

## [0.1.0] — 2026-06-20

The first release. A deterministic CLOB core with the pipeline
`Gateway → Sequencer → Matching Engine → Output`.

### Added

- **Pipeline `Clob`** — a single entry point `submit()` / `submit_into()`, tying all the stages together.
- **Gateway** — order validation: rejection on zero quantity (`ZeroQuantity`) and zero price of a limit order (`InvalidPrice`).
- **Sequencer** — monotonic sequence numbers and order identifiers for a deterministic order.
- **Matching Engine** — matching by price-time priority (FIFO within a price level).
- **Order book** — a `BTreeMap` of price levels on each side, a `VecDeque` for FIFO within a level, a `HashMap` order index for cancellation.
- **Order types** — `Limit`, `Market`.
- **Time-in-force** — `Gtc`, `Ioc`, `Fok`.
- **Events** — `Accepted`, `Trade`, `Resting`, `Filled`, `Canceled`, `Rejected`.
- **Book access** — `best_bid`, `best_ask`, `spread`, `depth`, `len`, `contains`, `available_qty`.
- **Examples** — `examples/basic.rs` (a demonstration of events and the book), `examples/throughput.rs` (a load run).
- **Tests** — 12 integration tests for matching, priority, TIF, cancellation and book invariants.
- **Documentation** — the `docs/` folder (status, architecture, changelog, roadmap).

### Implementation notes

- Integer prices and quantities (`u64`), without floating-point numbers.
- A core without external dependencies (only `std`).
- Rust edition 2024; a `release` profile with LTO and `panic = "abort"`.

[Unreleased]: https://example.com/clob/compare/v0.1.0...HEAD
[0.1.0]: https://example.com/clob/releases/tag/v0.1.0
