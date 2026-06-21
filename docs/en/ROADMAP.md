# Roadmap

The stages are approximate, the order may change. Markers: `[ ]` — not started,
`[~]` — in progress, `[x]` — done. Current status — in [STATUS.md](STATUS.md).

## Crate scope

`clob` is a pure embeddable matching engine: `std` only, a deterministic
single-threaded core, events on output. The core is already a **finished,
tested block** (stages v0.2–v0.4 are closed). Further
work in the crate itself — only domain completeness on `std`.

## v0.2 — Performance and order types

- [x] **Intrusive list at the price level** → cancel within a level in `O(1)`
      (nodes in a slab arena with a free-slot list; the former `O(n)` scan removed).
- [x] **`Modify` / amend command** — changing price/quantity; a price change or
      a volume increase loses time priority, a decrease preserves it.
- [x] **Post-only** — reject the order if it would immediately cross the spread (maker-only).
- [x] **Stop / stop-limit** — deferred orders, activated at the trigger price
      (by the last trade price; triggering is cascading, the `Triggered` event).
- [x] **Iceberg** — a visible (peak) and a hidden (reserve) part of the volume; when the
      visible part is exhausted the peak is replenished from the reserve to the tail of the level (losing priority).
- [x] A pool of reusable allocations in matching (a slab with a free-slot list;
      the `Vec` for executed ids removed).

## v0.3 — Reliability and replay

- [x] **Event journal (event sourcing)** — recording the input command stream into an append-only
      WAL (`Journal`): a custom binary format with varint, per-frame CRC32, group
      commit with `fsync`. The `PersistentClob` wrapper journals the command write-ahead — durable
      before events are returned.
- [x] **Replay** — restoring state by re-feeding the journal into a fresh `Clob`
      (`PersistentClob::open`); thanks to determinism the book, `order_id` and `seq` are reproduced
      exactly. A corrupt tail from an interrupted write is cut off by CRC. A hot standby (replica) —
      via the same command-stream reading mechanism.
- [x] **Snapshots** of the book state to speed up recovery (`checkpoint()`
      writes a snapshot of the full state and rotates the journal; `open` loads the snapshot and replays
      only the journal tail; reconciliation is idempotent by `seq`, the checkpoint is crash-safe). *(phase 2 of v0.3)*
- [x] Format versioning: the journal — `magic "CLBW"` + version in the segment header;
      snapshots — `magic "CLBS"` + version, a common frame with CRC32.

## v0.4 — Market data

- [x] **L2 snapshot** (aggregated depth by level) and **L3** (by order) —
      `Clob::l2_snapshot` / `Clob::l3_snapshot`, both tagged with `seq`; a read projection in
      `src/marketdata.rs`, the hidden reserve of icebergs does not make it into the snapshots.
- [x] **Incremental L2 updates** (market-by-price): deltas of only the changed
      levels (`L2Update` / `L2Feed`) on top of a snapshot with a common `seq`; the core is not touched.
- [x] **Incremental L3 updates** (market-by-order, with queue position): per-order
      deltas `L3Delta` (`Added` / `Reduced` / `Removed`) in `L3Update` via `L3Feed`
      (`src/l3feed.rs`). The affected levels are derived from events (as with L2), and the queue order
      within a level is reconciled against the book (`OrderBook::level_orders`); iceberg replenishment and loss of
      priority on amend are expressed as remove-then-add (a move to the tail of the level).
- [x] **Trade tape** as a separate channel — `TradeTape` extracts public anonymized prints
      from the event stream (`TapeTrade { seq, price, qty, taker_side }`, one per
      `Trade` event) and keeps a bounded ring history. The channel is **book-free**: `apply` takes only
      events, without access to the book. A print carries the command's `seq` — a common anchor with the book channel.
- [x] **Sequence-number consistency between data channels** — all channels (L2/L3
      snapshots, the `L2Update` / `L3Update` increments, the `TapeTrade` prints) are anchored on the command's `seq`
      via a single `command_seq`: for one command all channels carry one `seq`, equal to
      `Clob::current_seq()` after it. A consumer applies increments with a `seq` greater than that of the
      snapshot, and matches the channels with each other by the common `seq`.

## v0.5 — Domain completeness of the engine (`std`-only)

The domain logic of the engine itself — without external dependencies, the core stays pure and
deterministic.

- [x] **Order account/owner** and **self-trade prevention**:
      `owner: AccountId` + `stp: StpMode` in the order model; STP fires on matching by the
      **aggressor's** policy (cancel-taker / cancel-maker / cancel-both), per each resting maker.
      `owner` survives snapshot/replay and does not leak into market data. The `decrement` policy (trimming both
      by the overlap) and STP on `Modify` — deferred.
- [x] **Pre-trade risk** in the Gateway: `RiskConfig` (tick/lot, price band, position limit),
      hooked in via `Clob::with_risk` / `PersistentClob::open_with_risk`, default off.
      Tick/lot — price/volume multiples; the band — `±band_ticks` from the mid `(best_bid+best_ask)/2`
      (skipped on a cold book; stops/market bypass it); the position limit — net-signed worst-case
      (`net + open volume of the side + qty ≤ limit`), only for `owner != 0`. Accounting of net/open
      volume is kept by the book; the net survives the snapshot (`CLBS` → v3) and replay. STP-`Modify` and fees — separate.
- [x] **Fees** maker/taker and their computation in trade events: `FeeConfig` (maker/taker rates
      in **ppm** — fractions per million of the notional `price·qty`), **signed** (`i64`) — a negative
      maker rate = a rebate. Hooked in via `Clob::with_fees` / `with_risk_and_fees` /
      `PersistentClob::open_with_fees` / `open_with_risk_and_fees`, default off. The fee is computed
      on each trade and output in `Event::Trade` (`taker_fee` / `maker_fee`); rounding is
      truncation toward zero. The core does **not** accumulate fee state (events out, not state in), the snapshot does
      not change, `FeeConfig` is not persisted (on replay — the same config). Stage v0.5 is closed.

## Core release

The core is a finished block, a logical point for the first public release (a candidate for `1.0`).

- [ ] Bump the version in `Cargo.toml`, set a git tag, align the links in
      [CHANGELOG.md](CHANGELOG.md).
- [ ] **Rustdoc** across the entire public API + publication on docs.rs.
- [ ] Publish on crates.io (the manifest is already ready: description / keywords /
      categories / license).
- [ ] **License files** `LICENSE-MIT` and `LICENSE-APACHE`.

## Quality and tooling

These items pull in external crates, so they are **not** part of the core dependencies — they live as
`std`-only examples or a separate dev harness (the zero-dependency rule in `CLAUDE.md`).

- [ ] Latency benchmarks (p50/p99/p99.9) — as `examples/` on `std::time`
      (we do not add criterion as a dependency).
- [ ] **Property-based tests** and **fuzzing** of the command pipeline — as a separate dev harness
      outside the crate (proptest / fuzz pull in dependencies).
