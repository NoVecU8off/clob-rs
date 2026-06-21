# Status

**Version:** 0.1.0
**Date:** 2026-06-20
**Maturity:** early (alpha). The core works and is covered by tests; not intended for production — persistence is basic (journal + replay + snapshots), a number of order types are missing (see [ROADMAP.md](ROADMAP.md)).

## At a glance

- Quality: passes the entire done-gate — `cargo build --all-targets`, `cargo clippy -- -D warnings`, `cargo fmt --check` with no complaints.
- Tests: **169 integration tests** pass (`cargo test`) — matching/TIF/market/post-only ([matching.rs](../../tests/matching.rs)), amend ([modify.rs](../../tests/modify.rs)), stop/stop-limit and cascade ([stops.rs](../../tests/stops.rs)), iceberg — visible peak, replenishment and loss of priority ([iceberg.rs](../../tests/iceberg.rs)), cancellation and the intrusive list ([cancel.rs](../../tests/cancel.rs)), book accessors and determinism ([book.rs](../../tests/book.rs)), persistence — command round-trip, replay/recovery, corrupt tail, versioning ([persistence.rs](../../tests/persistence.rs)), snapshots/checkpoint, journal rotation and crash-safe recovery by `seq` ([snapshot.rs](../../tests/snapshot.rs)), L2/L3 market snapshots — aggregation, queue order, hiding the iceberg reserve, L2↔L3 consistency ([marketdata.rs](../../tests/marketdata.rs)), incremental L2 deltas — adding/trimming/removing levels, modify-reprice, iceberg replenishment, stop cascade and step-by-step convergence of the mirror with a live snapshot ([incremental.rs](../../tests/incremental.rs)), incremental L3 deltas — add/reduce/remove, iceberg replenishment with re-insertion at the tail, modify-reprice and loss of priority, step-by-step convergence of the mirror with a live L3 snapshot ([l3incremental.rs](../../tests/l3incremental.rs)), trade tape — prints from the event stream, taker-aggressor, the full executed volume of an iceberg (including the hidden part), ring history and its bounding ([tape.rs](../../tests/tape.rs)), end-to-end sequencing of data channels — a single `seq` across snapshots/increments/tape and alignment of a late consumer ([channels.rs](../../tests/channels.rs)), self-trade prevention — three policies (cancel-taker / cancel-maker / cancel-both), cross-owners, anonymous, off, a fired stop and survival of `owner` through snapshot+replay ([stp.rs](../../tests/stp.rs)), pre-trade risk — tick/lot (incl. iceberg `display`, market exemption), price band from mid scaled by tick (cold book, stop/market exemption), worst-case position limits (open volume, net from trades, their sum, side symmetry, anonymous, exemption on cancel, modify by own contribution, iceberg reserve, STP-`detach`) and survival of the net position through checkpoint and replay ([risk.rs](../../tests/risk.rs)), maker/taker fees — ppm of notional, signed maker rebate, truncation toward zero, zero default fee, per-trade computation by levels, market taker, the full volume of an iceberg (including the hidden part), composition with risk and application via `PersistentClob` ([fees.rs](../../tests/fees.rs)); shared helpers in [tests/common](../../tests/common/mod.rs).
- Dependencies: **none** — only `std`.
- Performance: **~31 million orders/s** on a synthetic benchmark (release, single thread, `cargo run --release --example throughput`). The figure depends on hardware and scenario and is indicative only.
- Rust edition: 2024.

## What's implemented

| Subsystem | Status | Details |
| --- | --- | --- |
| Input API | ✅ | `Command::New` / `Command::Cancel` / `Command::Modify`, constructors `NewOrder::limit` / `market` / `stop` / `stop_limit` / `iceberg`, builders `.with_tif()` / `.with_owner()` / `.with_stp()`, `ModifyOrder::new` |
| **Gateway** | ✅ | Validation: rejection on zero quantity and zero price — limit (`price`), stop/stop-limit (`trigger`), stop-limit (also `price`); the same checks for `Modify`. **Pre-trade risk** (opt-in `RiskConfig`, default off): tick/lot, price band from mid, position limits (net worst-case) |
| **Sequencer** | ✅ | Monotonic `seq` and `order_id`; deterministic order |
| **Matching Engine** | ✅ | Matching by price-time priority; market/limit; TIF `Gtc` / `Ioc` / `Fok` / `PostOnly`; amend (`Modify`); stop/stop-limit with cascade activation; iceberg (visible peak + reserve replenishment with loss of priority); self-trade prevention (`StpMode` cancel-taker / maker / both, taker governs); maker/taker fees (`FeeConfig`, ppm of notional, signed rebate) on each trade |
| **Order book** | ✅ | `BTreeMap` of levels per side, intrusive doubly-linked FIFO list within a level (nodes in a slab arena), `HashMap` index → cancel in `O(1)`; a side index of hidden iceberg reserves; a per-account index of net position (`i128`) and open volume per side for risk checks (`src/book/accounts.rs`) |
| **Stop book** | ✅ | Dormant stop orders outside the main book; activation by last trade price; `Clob::pending_stops()` |
| **Output** | ✅ | Events `Accepted`, `Trade`, `Resting`, `Filled`, `Canceled`, `Modified`, `Triggered`, `Rejected`; `Trade` carries `taker_fee` / `maker_fee` (`i64`, signed) |
| **Persistence** | ✅ | Command WAL + snapshots (`PersistentClob`, `Journal`): binary journal with varint + CRC32, write-ahead `fsync`, replay recovery; `checkpoint()` writes a state snapshot and rotates the journal → recovery loads the snapshot and replays only the tail; format versioning (`CLBW` journal / `CLBS` snapshot) |
| Market data | ✅ | Snapshots `l2_snapshot()` (aggregated depth) and `l3_snapshot()` (per order, without the hidden iceberg reserve), both tagged with `seq`; incremental updates L2 (`L2Update` / `L2Feed`, level deltas) and L3 (`L3Update` / `L3Feed` / `L3Delta`, per-order deltas with queue position); trade tape `TradeTape` / `TapeTrade` (book-free stream of prints); all channels are anchored to a single command `seq` (`command_seq`); plus `depth()` / `level_qty()` / `level_orders()` / `best_bid` / `best_ask` / `spread` |
| Examples | ✅ | `examples/basic.rs`, `examples/throughput.rs`, `examples/replay.rs` |

## Order types and time-in-force

- Types: `Limit`, `Market`, `Stop` (stop-market), `StopLimit` (stop-limit), `Iceberg` (visible peak + hidden reserve).
- Time-in-force: `Gtc` (rests in the book), `Ioc` (execute now, cancel the remainder), `Fok` (execute in full or reject), `PostOnly` (maker only: reject with `WouldCross` if it would immediately cross the spread).

## Modifying orders (amend)

- `Modify` changes the price and/or quantity of a resting order, preserving `order_id`.
- Reducing the quantity at the same price preserves priority (in-place edit, `O(1)`).
- Changing the price or increasing the quantity — loss of priority: the order is removed and re-matched (executes if crossing the spread).

## Stop orders

- `Stop` (stop-market) and `StopLimit` (stop-limit) "sleep" in a separate book outside the main one and are not visible in `depth()` / `len()`; their count is in `Clob::pending_stops()`.
- The trigger is the **last trade price**: a buy-stop activates when `last >= trigger`, a sell-stop when `last <= trigger`. If the market is already past the trigger, the stop fires immediately on acceptance.
- On firing, a `Triggered` is emitted, then the order goes through ordinary matching: `Stop` — as a market order, `StopLimit` — as a limit at its own price subject to TIF (incl. `Fok` / `PostOnly` prechecks at the moment of activation).
- Activation is **cascading**: the trades of a fired stop move the price and may activate the next stops (order — by acceptance time). The cascade is deterministic and finite.
- `Cancel` removes a dormant stop; `Modify` of a dormant stop is not yet supported.

## Iceberg

- `Iceberg` shows in the book only the visible part (peak, `display`); the rest of the volume lies in the hidden reserve and is not visible in `depth()` / `len()` / `available_qty()`.
- When the visible peak is fully matched, a new peak `min(display, hidden)` is replenished from the reserve and rests **at the tail** of its price level — behind all already-displayed orders (loss of time priority).
- A sufficiently large opposite order matches the whole iceberg in one pass (layer by layer, including the hidden part); priority is lost only relative to other orders at the level.
- As a taker the iceberg matches for the full `qty` (like an ordinary limit); `display` applies only to the remainder that rests in the book. `display >= qty` → an ordinary fully visible order. The meaningful TIFs are `Gtc` and `PostOnly`.
- Replenishment emits no events (visible in `depth()`); `Cancel` removes the iceberg in its entirety together with the reserve; `Modify` treats `qty` as the new full volume, preserves `display` and re-inserts the order (losing priority).

## Account/owner and self-trade prevention (STP)

- An order carries an optional `owner: AccountId` (a `u64` primitive; `0` — anonymous / not set) and `stp: StpMode` (`Off` | `CancelTaker` | `CancelMaker` | `CancelBoth`); they are set by the builders `NewOrder::with_owner(id)` / `with_stp(mode)`, with defaults `owner = 0`, `stp = Off`. STP is strictly opt-in: default orders behave as before.
- For the core, `owner` is an **opaque equality token**: the engine does not interpret it, only compares it; the core accepts the stamped `owner` as trusted.
- STP is checked **at the moment of matching, for each resting maker**: when `taker.stp != Off && taker.owner != 0 && taker.owner == maker.owner`, instead of a trade the **aggressor's** policy is applied (taker governs) — the maker's `stp` is not read. `CancelTaker` removes the aggressor's remainder (the maker is intact); `CancelMaker` removes the encountered resting order and the aggressor continues down the queue; `CancelBoth` removes both. Foreign liquidity is matched normally meanwhile. All STP removals emit `Canceled`.
- STP also applies to a **fired stop**: it carries its own `owner` / `stp` and on activation acts as an aggressor.
- `owner` is stored inline on the resting order (`RestingOrder`), **survives snapshot and replay** (journal `CLBW` version `2`, snapshot `CLBS` version `3`) and **does not leak** into the public market data projections (L2/L3/tape remain anonymized).

## Pre-trade risk (tick/lot, band, position limits)

- Enabled via `Clob::with_risk(RiskConfig)` / `PersistentClob::open_with_risk(path, cfg)`; `Clob::new()` and `PersistentClob::open` remain **without checks** (risk control strictly opt-in, default off). `RiskConfig` is assembled by the builders `with_tick` / `with_lot` / `with_price_band(band_ticks)` / `with_position_limit`; a value of `0`/`1` disables the check.
- **Tick/lot.** The price is a multiple of `tick_size` (limit price, stop `trigger`, both fields of a stop-limit, iceberg price); `qty` and the iceberg `display` are multiples of `lot_size`. Market skips the price checks (there is no price), the lot applies to `qty`.
- **Price band.** A window `[mid − band_ticks·tick, mid + band_ticks·tick]` around `mid = (best_bid + best_ask) / 2` (tick = `max(tick_size, 1)`). On a cold/one-sided book (mid undefined) the band is skipped; stops (rest outside the market) and market — pass. The reference is taken from the book, but the Gateway does not read it itself — `Clob` feeds the mid into the context (stage separation preserved).
- **Position limits (net-signed, worst-case).** Only for `owner != 0` (anonymous is exempt). Buy: `net + open_buy_volume + qty ≤ limit`; sell: `−net + open_sell_volume + qty ≤ limit`. Open volume accounts for the full remainder of an iceberg (visible + hidden reserve). On `Modify` the order's own current contribution is subtracted, then the new `qty` is added.
- **The book keeps the accounting** (single source of truth): `match_against` sees both participants of a trade, so the net position (`i128`) and the open volume are updated within the book on `insert` / `cancel` / `reduce` / matching / STP-`detach`; `Clob`/the engine only read the aggregates for validation. The net position **survives** the snapshot (`CLBS` version `3`, net section, accounts sorted by `AccountId`) and replay; the journal (`CLBW`) does not change. `RiskConfig` is not persisted — the application sets it at open time (on replay — the **same** config, otherwise history will diverge).
- New `RejectReason`s: `TickSize`, `LotSize`, `PriceBand`, `PositionLimit`.

## Fees (maker/taker)

- Enabled via `Clob::with_fees(FeeConfig)` / `Clob::with_risk_and_fees(risk, fees)` and `PersistentClob::open_with_fees(path, fees)` / `open_with_risk_and_fees(path, risk, fees)`; `Clob::new()` and the previous constructors remain **without fees** (default off). `FeeConfig` is assembled by the builders `with_maker_ppm` / `with_taker_ppm`; a rate of `0` disables the side.
- **The model — ppm of notional.** `fee = price · qty · rate_ppm / 1_000_000` (intermediate `i128` with saturation, rounding by **truncation toward zero** — symmetric for fee and rebate). The rates are **signed** (`i64`): positive — a fee, **a negative maker rate is a rebate** (a payout to the maker, a negative `maker_fee`).
- **Where it is computed.** On each trade at the point of its birth (the engine, the `on_trade` callback, where both participants are visible). The **entire executed volume** is charged, including the hidden iceberg reserve — a print on each matched layer. STP cancellations and other non-factual events carry no fee.
- **Output — in the event.** Two new fields of `Event::Trade`: `taker_fee: i64` and `maker_fee: i64`. By the principle of "events out, not state in" the core **does not accumulate** fees. Therefore **the snapshot does not change** (`CLBS` stays version `3`), the journal is untouched, and `FeeConfig` is **not persisted** — the application sets it at open time (fees do not affect the book's state, only the output numbers).
- **Anonymity.** `taker_fee` / `maker_fee` are private and **do not reach** the public market data channels (L2/L3 snapshots, increments, the `TapeTrade` tape).

## Market data (snapshots, incremental updates and the trade tape)

- `Clob::l2_snapshot(depth)` — **market-by-price**: aggregated depth by levels (`L2Snapshot`: `bids` / `asks` of `L2Level { price, qty }`), best price first, no more than `depth` levels per side.
- `Clob::l3_snapshot()` — **market-by-order**: the book by individual orders (`L3Snapshot`: `bids` / `asks` of `L3Order { id, side, price, qty }`), best-first by price and FIFO within a level — that is, the queue position is visible.
- Both snapshots carry the current `seq` — an anchor for future incremental updates.
- The projection is **public and anonymized**: for an iceberg only the visible peak is given, the hidden reserve does not reach the snapshots (privacy is determined by what a participant *posted*, not by the granularity of the feed); an order has no owner/account in the model yet.
- Implemented as a read projection (`src/marketdata.rs`) over `depth()` / `resting_orders()` — the deterministic core is untouched.
- **Incremental L2 updates.** `L2Feed::apply(&events, book)` yields an `L2Update` — deltas of only the changed price levels of one command (a level with `qty == 0` — removal), with the same `seq`. The consumer takes `l2_snapshot` (or `L2Feed::from_book`) as a base and applies the deltas, without resending the whole book. The affected levels are derived from the events, the new volume is read from the book (`OrderBook::level_qty`), so the deltas are accurate even with iceberg replenishment and with modify-reprice. The hidden iceberg reserve does not reach the deltas.
- **Incremental L3 updates (market-by-order).** `L3Feed::apply(&events, book)` yields an `L3Update` — per-order deltas `L3Delta` (`Added { id, side, price, qty }` at the tail of a level / `Reduced { id, qty }` preserving position / `Removed { id }`), with the same `seq`. The affected levels are derived from the events (as with L2), but the queue order within a level is checked against the book (`OrderBook::level_orders`): the deltas are a diff of the old level list against the new one, ordered `Removed` → `Reduced` → `Added`. Therefore iceberg replenishment and loss of priority on amend are expressed as remove-then-add (re-insertion at the tail) and exactly reproduce the book's FIFO. `L3Feed::from_book` seeds the feed from the `l3_snapshot` snapshot with the same `seq`. The hidden iceberg reserve does not reach the deltas.
- **End-to-end sequencing of channels.** All market data channels are anchored to the command `seq` via a single `command_seq`: snapshots carry `current_seq()`, the increments `L2Update` / `L3Update` and the prints `TapeTrade` — the same command `seq`. For one command all channels carry one `seq`, so the consumer applies increments with a `seq` greater than the snapshot's and reconciles the channels with one another by the common anchor.
- **Trade tape.** `TradeTape::apply(&events)` extracts from the event stream public anonymized prints `TapeTrade { seq, price, qty, taker_side }` — one per each `Event::Trade`, in execution order; returns the command's prints and accumulates them in a bounded ring history (`new()` — without a limit, `bounded(cap)` — the last `cap`; reading — `recent()` / `last()` / `len()` / `is_empty()`). Unlike the L2/L3 projections the tape is **book-free** — `apply` takes only `&[Event]` and does not read the book, so it can be maintained by a consumer that sees only the event stream. A print carries the command `seq` — a common anchor with the book channel. Anonymity is about the owner and identifiers (order ids do not reach the tape); the executed volume, conversely, is public and is printed **in full**: a pass over an iceberg gives a print on each matched layer, including the volume from the hidden reserve (the book hides the *resting* reserve, the tape prints the *executed*).

## Persistence (journal, snapshots and replay)

- `PersistentClob` wraps `Clob` and keeps a write-ahead journal of input commands: a command is serialized and flushed to disk (`fsync`) **before** the events are returned — an acknowledged order survives a process crash.
- Recovery (`PersistentClob::open`) is a **replay**: the journal is re-fed into a fresh `Clob`; thanks to determinism the book, `order_id` and `seq` are reproduced exactly. Commands rejected by the gateway are also journaled (they consume `seq`).
- The journal format is a custom binary one (`std`-only): a segment header `magic "CLBW"` + version, command records with varint fields, blocks framed by length + CRC32. Group commit — `submit_batch` (one `fsync` per batch). An interrupted last record is cut off by CRC.
- **Snapshots.** `checkpoint()` writes a snapshot of the full deterministic state (sequencer counters `seq`/`next_order_id`, last trade price, resting orders in priority order together with the hidden iceberg reserves, dormant stops, accounts' net positions) and rotates the journal, leaving only the tail. `open` loads the snapshot and replays **only the journal commands after the snapshot** — recovery does not replay the whole stream. The snapshot format is `magic "CLBS"` + version, varint fields, a common frame with CRC32 (the same codec). The write is atomic: temporary file → `fsync` → `rename`.
- **Checkpoint crash safety.** Reconciliation of the snapshot and the journal is idempotent by `seq`: on recovery only commands with a `seq` greater than the one covered by the snapshot are applied. Therefore a checkpoint interrupted midway (the snapshot written, the journal not yet rotated) gives neither double application nor loss of commands. A corrupt snapshot (CRC does not match) → `JournalError::CorruptSnapshot`.
- Low-level access: `Journal` (writer), `read_commands` (reader of the command stream).

## Known limitations

- **Stops are triggered by the last trade price** (not by quotes); dormant stops are not reflected in depth/`len()`; amend of a dormant stop is not supported.
- **Iceberg**: the hidden reserve is not visible in `depth()` / `len()` / `available_qty()` and is not accounted for in the `Fok` liquidity check; replenishment of the visible part does not produce events; `Modify` of an iceberg always re-inserts the order (losing priority).
- **Self-trade prevention** is checked only on `New`: `Modify` (amend) is re-matched without STP; the `Fok` / post-only prechecks count the opposite volume in full, without subtracting the owner's own liquidity. `owner` is private — it does not reach L2/L3 snapshots, increments and the trade tape. The `decrement` policy (trimming both on overlap) is not yet implemented.
- **Persistence — journal + snapshots + replay** (`PersistentClob`): the snapshot and journal rotation are triggered only by an explicit `checkpoint()` (there are no automatic checkpoints by timer/volume — that is the application's decision, the core stays without a clock); per-command `fsync` is slow (for throughput — group commit `submit_batch`); an `fsync` failure in the middle of the stream should be considered fatal (restart and recovery from the journal).
- **Market data**: L2/L3 snapshots, incremental deltas L2 (`L2Feed`) and L3 (`L3Feed` — market-by-order with queue position) and the trade tape (`TradeTape`); all channels are anchored to a single command `seq`. The book feeds are co-located — `apply` reads the book; the tape, conversely, is **book-free** — the consumer maintains it from a single event stream. (Stage v0.4 is closed.)
- **Pre-trade risk** (opt-in `RiskConfig`): tick/lot, price band from mid, position limits (net worst-case). Dormant stops are not accounted for in the open volume until they fire; `RiskConfig` is not persisted — the application sets it at open time (on replay — the same one).
- **Fees** maker/taker (opt-in `FeeConfig`): rates in ppm of notional, signed (maker rebate). Computed on each trade and output in `Event::Trade` (`taker_fee` / `maker_fee`); the core **does not accumulate** them, `FeeConfig` is **not persisted**. Rounding — truncation toward zero.
- **Logical time**: `timestamp` is equated to `seq` (a separate clock is not introduced for the sake of determinism).

## How to verify

```sh
cargo test
cargo run --example basic
cargo run --example replay
cargo run --release --example throughput
```
