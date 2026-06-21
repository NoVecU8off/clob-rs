# 当前状态

**版本：** 0.1.0
**日期：** 2026-06-20
**成熟度：** 早期（alpha）。核心可用并有测试覆盖；不适用于生产环境——持久化是基础级别（日志 + 重放 + 检查点），缺少若干订单类型（见 [ROADMAP.md](ROADMAP.md)）。

## 概要

- 质量：通过全部 done-gate——`cargo build --all-targets`、`cargo clippy -- -D warnings`、`cargo fmt --check` 均无任何告警。
- 测试：**169 个集成测试**通过（`cargo test`）——撮合/TIF/market/post-only（[matching.rs](../../tests/matching.rs)）、amend（[modify.rs](../../tests/modify.rs)）、止损/止损限价及级联（[stops.rs](../../tests/stops.rs)）、iceberg——可见 peak（显示部分）、补充与丧失优先级（[iceberg.rs](../../tests/iceberg.rs)）、撤单与侵入式链表（[cancel.rs](../../tests/cancel.rs)）、订单簿访问器与确定性（[book.rs](../../tests/book.rs)）、持久化——命令往返、重放/恢复、损坏的尾部、版本控制（[persistence.rs](../../tests/persistence.rs)）、检查点、日志轮转以及按 `seq` 的崩溃安全恢复（[snapshot.rs](../../tests/snapshot.rs)）、L2/L3 行情快照——聚合、队列顺序、隐藏冰山储备、L2↔L3 一致性（[marketdata.rs](../../tests/marketdata.rs)）、增量 L2 增量更新——价位的添加/截断/删除、modify 重定价、冰山补充、止损级联以及镜像与活动快照的逐步收敛（[incremental.rs](../../tests/incremental.rs)）、增量 L3 增量更新——add/reduce/remove、冰山补充并重排到队尾、modify 重定价与丧失优先级、镜像与活动 L3 快照的逐步收敛（[l3incremental.rs](../../tests/l3incremental.rs)）、成交带——来自事件流的成交记录、taker 主动方、冰山完整成交量（含隐藏部分）、环形历史及其上限（[tape.rs](../../tests/tape.rs)）、data 通道的端到端序列编号——快照/增量/成交带共享同一 `seq` 以及滞后消费者的对齐（[channels.rs](../../tests/channels.rs)）、self-trade prevention——三种策略（cancel-taker / cancel-maker / cancel-both）、跨所有者、匿名、off、已触发止损以及 `owner` 经检查点+重放后的存续（[stp.rs](../../tests/stp.rs)）、pre-trade risk——tick/lot（含冰山 `display`、market 的豁免）、以 tick 为刻度的 mid 价格带（冷订单簿、止损/market 的豁免）、最坏情况持仓限额（敞口量、来自成交的净额、两者之和、买卖方对称、匿名、撤单时的释放、按自身贡献的 modify、冰山储备、STP-`detach`）以及净持仓经 checkpoint 和重放后的存续（[risk.rs](../../tests/risk.rs)）、maker/taker 手续费——名义金额的 ppm、挂单方的带符号返佣、向零截断、零默认手续费、按价位的逐笔成交计算、市价 taker、冰山完整成交量（含隐藏部分）、与风控的组合以及通过 `PersistentClob` 的应用（[fees.rs](../../tests/fees.rs)）；通用辅助函数在 [tests/common](../../tests/common/mod.rs)。
- 依赖：**无**——仅 `std`。
- 性能：在合成基准上**约 3100 万订单/秒**（release，单线程，`cargo run --release --example throughput`）。该数字取决于硬件和场景，仅供参考。
- Rust 版次：2024。

## 已实现内容

| 子系统 | 状态 | 详情 |
| --- | --- | --- |
| 输入 API | ✅ | `Command::New` / `Command::Cancel` / `Command::Modify`，构造函数 `NewOrder::limit` / `market` / `stop` / `stop_limit` / `iceberg`，构建器 `.with_tif()` / `.with_owner()` / `.with_stp()`，`ModifyOrder::new` |
| **Gateway** | ✅ | 验证：数量为零和价格为零时拒绝——限价（`price`）、止损/止损限价（`trigger`）、止损限价（还包括 `price`）；`Modify` 的相同检查。**Pre-trade risk**（可选启用 `RiskConfig`，默认关闭）：tick/lot、以 mid 为基准的价格带、持仓限额（最坏情况净额） |
| **Sequencer** | ✅ | 单调递增的 `seq` 和 `order_id`；确定性顺序 |
| **Matching Engine** | ✅ | 按价格-时间优先撮合；market/limit；TIF `Gtc` / `Ioc` / `Fok` / `PostOnly`；amend（`Modify`）；带级联激活的止损/止损限价；iceberg（可见 peak（显示部分）+ 从储备补充并丧失优先级）；self-trade prevention（`StpMode` cancel-taker / maker / both，由吃单方决定，taker governs）；每笔成交上的 maker/taker 手续费（`FeeConfig`、名义金额的 ppm、带符号返佣） |
| **Order book** | ✅ | 按买卖方组织的 `BTreeMap` 价位、价位内的侵入式双向 FIFO 链表（节点在 slab 内存池中）、`HashMap` 索引 → `O(1)` 撤单；冰山隐藏储备的旁路索引；用于风控检查的按账户净持仓索引（`i128`）和按买卖方的敞口量（`src/book/accounts.rs`） |
| **止损单簿** | ✅ | 主订单簿之外的休眠止损单；按最后成交价激活；`Clob::pending_stops()` |
| **Output** | ✅ | 事件 `Accepted`、`Trade`、`Resting`、`Filled`、`Canceled`、`Modified`、`Triggered`、`Rejected`；`Trade` 携带 `taker_fee` / `maker_fee`（`i64`，带符号） |
| **持久化** | ✅ | 命令的 WAL + 检查点（`PersistentClob`、`Journal`）：带 varint + CRC32 的二进制日志、预写 `fsync`、重放恢复；`checkpoint()` 写入状态快照并轮转日志 → 恢复时加载快照并只重放尾部；格式版本控制（`CLBW` 日志 / `CLBS` 快照） |
| Market data | ✅ | 快照 `l2_snapshot()`（聚合深度）和 `l3_snapshot()`（按订单，不含冰山隐藏储备），两者均标记 `seq`；L2 增量更新（`L2Update` / `L2Feed`，价位增量）和 L3 增量更新（`L3Update` / `L3Feed` / `L3Delta`，带队列位置的逐订单增量）；成交带 `TradeTape` / `TapeTrade`（无需订单簿的成交记录流）；所有通道锚定到同一条命令的 `seq`（`command_seq`）；外加 `depth()` / `level_qty()` / `level_orders()` / `best_bid` / `best_ask` / `spread` |
| 示例 | ✅ | `examples/basic.rs`、`examples/throughput.rs`、`examples/replay.rs` |

## 订单类型与 time-in-force

- 类型：`Limit`、`Market`、`Stop`（止损市价单）、`StopLimit`（止损限价单）、`Iceberg`（可见 peak（显示部分）+ 隐藏储备）。
- Time-in-force：`Gtc`（进入订单簿）、`Ioc`（立即成交，剩余量撤销）、`Fok`（全部成交或拒绝）、`PostOnly`（仅挂单方：若会立即穿越价差则以 `WouldCross` 拒绝）。

## 订单修改（amend）

- `Modify` 修改在簿订单的价格和/或数量，保留 `order_id`。
- 同价位下减少数量保留优先级（原地修改，`O(1)`）。
- 改变价格或增加数量——丧失优先级：订单被撤下并重新撮合（穿越价差时成交）。

## 止损单

- `Stop`（止损市价单）和 `StopLimit`（止损限价单）在主订单簿之外的独立簿中「休眠」，不出现在 `depth()` / `len()` 中；其数量在 `Clob::pending_stops()` 中。
- 触发依据是**最后成交价**：buy-stop 在 `last >= trigger` 时激活，sell-stop 在 `last <= trigger` 时激活。若市场已越过触发价，止损在接收时立即触发。
- 触发时发出 `Triggered`，随后订单走常规撮合：`Stop` 作为市价单，`StopLimit` 作为按自身价格并考虑 TIF 的限价单（含激活时刻的 `Fok` / `PostOnly` 预检查）。
- 激活是**级联的**：已触发止损的成交推动价格并可能激活后续止损（顺序按接收时间）。级联是确定性且有限的。
- `Cancel` 撤下休眠止损单；休眠止损单的 `Modify` 暂不支持。

## Iceberg

- `Iceberg` 在订单簿中只显示可见部分（peak，`display`）；剩余量位于隐藏储备中，不出现在 `depth()` / `len()` / `available_qty()` 中。
- 当可见 peak 完全撮合后，从储备补充一个新 peak `min(display, hidden)` 并排到其价位的**队尾**——位于所有已显示订单之后（丧失时间优先级）。
- 足够大的对手订单一次性撮合整个冰山（逐层，包括隐藏部分）；优先级仅相对于该价位的其他订单丧失。
- 作为 taker 时冰山按完整 `qty` 撮合（与普通限价单相同）；`display` 仅作用于进入订单簿的剩余量。`display >= qty` → 普通的完全可见订单。有意义的 TIF 是 `Gtc` 和 `PostOnly`。
- 补充不发出事件（在 `depth()` 中可见）；`Cancel` 连同储备一起撤下整个冰山；`Modify` 将 `qty` 视为新的完整数量，保留 `display` 并重新排放订单（丧失优先级）。

## 账户/所有者与 self-trade prevention (STP)

- 订单携带可选的 `owner: AccountId`（`u64` 原始类型；`0` 表示匿名 / 未设置）和 `stp: StpMode`（`Off` | `CancelTaker` | `CancelMaker` | `CancelBoth`）；通过构建器 `NewOrder::with_owner(id)` / `with_stp(mode)` 设置，默认 `owner = 0`、`stp = Off`。STP 严格可选启用：默认订单的行为与以往一致。
- 对核心而言，`owner` 是**不透明的相等性令牌**：引擎不解释它，只做比较；核心把盖章后的 `owner` 视为可信。
- STP 在**撮合时刻、针对每个对手挂单做市方 (maker)** 检查：当 `taker.stp != Off && taker.owner != 0 && taker.owner == maker.owner` 时，以**主动方策略（由吃单方决定，taker governs）**取代成交——不读取 maker 的 `stp`。`CancelTaker` 撤下主动方剩余量（maker 保持完整）；`CancelMaker` 撤下遇到的在簿订单，主动方继续沿队列前进；`CancelBoth` 撤下两者。其间他人流动性照常撮合。所有 STP 撤单均发出 `Canceled`。
- STP 同样应用于**已触发止损**：它携带自身的 `owner` / `stp`，在激活时作为主动方行事。
- `owner` 内联存储在在簿订单（`RestingOrder`）上，**经检查点与重放后存续**（日志 `CLBW` 版本 `2`，快照 `CLBS` 版本 `3`）且**不泄漏**到公开的 market data 投影（L2/L3/成交带保持匿名化）。

## Pre-trade risk（tick/lot、价格带、持仓限额）

- 通过 `Clob::with_risk(RiskConfig)` / `PersistentClob::open_with_risk(path, cfg)` 接入；`Clob::new()` 和 `PersistentClob::open` 保持**无检查**（风控严格可选启用，默认关闭）。`RiskConfig` 由构建器 `with_tick` / `with_lot` / `with_price_band(band_ticks)` / `with_position_limit` 组装；值为 `0`/`1` 关闭该项检查。
- **Tick/lot。** 价格是 `tick_size` 的整数倍（限价单价格、止损的 `trigger`、止损限价单的两个字段、冰山价格）；冰山的 `qty` 和 `display` 是 `lot_size` 的整数倍。Market 跳过价格检查（无价格），lot 对 `qty` 适用。
- **价格带。** 围绕 `mid = (best_bid + best_ask) / 2` 的窗口 `[mid − band_ticks·tick, mid + band_ticks·tick]`（tick = `max(tick_size, 1)`）。在冷/单边订单簿上（mid 未定义）跳过价格带；止损（在市场之外）和 market 略过。参考值取自订单簿，但 Gateway 本身不读取它——`Clob` 将 mid 传入上下文（保持 stage separation）。
- **持仓限额（带符号净额、最坏情况）。** 仅对 `owner != 0`（匿名豁免）。买入：`net + 买入敞口量 + qty ≤ limit`；卖出：`−net + 卖出敞口量 + qty ≤ limit`。敞口量计入冰山的完整剩余量（可见 + 隐藏储备）。在 `Modify` 时减去订单自身的当前贡献，然后加上新的 `qty`。
- **由订单簿记账**（唯一真相来源）：`match_against` 同时看到成交双方，因此净持仓（`i128`）和敞口量在订单簿内部于 `insert` / `cancel` / `reduce` / 撮合 / STP-`detach` 时更新；`Clob`/引擎只读取聚合值用于验证。净持仓**经**快照（`CLBS` 版本 `3`，净额段，账户按 `AccountId` 排序）和重放后存续；日志（`CLBW`）不变。`RiskConfig` 不持久化——应用在打开时指定它（重放时用**相同**配置，否则历史会发散）。
- 新增 `RejectReason`：`TickSize`、`LotSize`、`PriceBand`、`PositionLimit`。

## 手续费（maker/taker）

- 通过 `Clob::with_fees(FeeConfig)` / `Clob::with_risk_and_fees(risk, fees)` 以及 `PersistentClob::open_with_fees(path, fees)` / `open_with_risk_and_fees(path, risk, fees)` 接入；`Clob::new()` 和以往的构造函数保持**无手续费**（默认关闭）。`FeeConfig` 由构建器 `with_maker_ppm` / `with_taker_ppm` 组装；费率 `0` 关闭该方。
- **模型——名义金额的 ppm。** `fee = price · qty · rate_ppm / 1_000_000`（中间过程为带饱和的 `i128`，**向零截断**取整——对手续费和返佣对称）。费率为**带符号**（`i64`）：正值为手续费，**maker 费率为负值表示返佣**（向 maker 支付，`maker_fee` 为负）。
- **在何处计算。** 在每笔成交诞生之处（引擎，`on_trade` 回调，可见成交双方）。**全部成交量**计费，包括冰山隐藏储备——每个撮合层一条成交记录。STP 撤单及其他非实际成交事件不计手续费。
- **输出——在事件中。** `Event::Trade` 的两个新字段：`taker_fee: i64` 和 `maker_fee: i64`。依「事件在输出，而非状态在内部」原则，核心**不累计**手续费。因此**快照不变**（`CLBS` 仍为版本 `3`），日志不受影响，且 `FeeConfig` **不持久化**——应用在打开时指定它（手续费不影响订单簿状态，只影响输出的数字）。
- **匿名性。** `taker_fee` / `maker_fee` 为私有，**不进入**公开的 market data 通道（L2/L3 快照、增量、成交带 `TapeTrade`）。

## Market data（快照、增量更新与成交带）

- `Clob::l2_snapshot(depth)`——**按价格聚合 (market-by-price)**：按价位的聚合深度（`L2Snapshot`：`bids` / `asks` 由 `L2Level { price, qty }` 组成），最优价在前，每侧不超过 `depth` 个价位。
- `Clob::l3_snapshot()`——**按订单 (market-by-order)**：按单个订单的订单簿（`L3Snapshot`：`bids` / `asks` 由 `L3Order { id, side, price, qty }` 组成），按价格 best-first 且价位内 FIFO——即可见队列位置。
- 两个快照均携带当前 `seq`——未来增量更新的锚点。
- 投影是**公开且匿名化的**：对 iceberg 只给出可见 peak（显示部分），隐藏储备不进入快照（隐私由参与者*所挂出*的内容决定，而非馈送的粒度）；订单的所有者/账户在该模型中暂时不存在。
- 实现为 `depth()` / `resting_orders()` 之上的只读投影（`src/marketdata.rs`）——确定性核心不受影响。
- **增量 L2 更新。** `L2Feed::apply(&events, book)` 给出 `L2Update`——仅单条命令所改变价位的增量（`qty == 0` 的价位表示删除），携带相同的 `seq`。消费者以 `l2_snapshot`（或 `L2Feed::from_book`）作为基线并应用增量，无需重发整个订单簿。受影响的价位从事件推导，新的量从订单簿读取（`OrderBook::level_qty`），因此增量在冰山补充和 modify 重定价时都精确。冰山隐藏储备不进入增量。
- **增量 L3 更新（按订单 market-by-order）。** `L3Feed::apply(&events, book)` 给出 `L3Update`——逐订单增量 `L3Delta`（`Added { id, side, price, qty }` 到价位队尾 / `Reduced { id, qty }` 保留位置 / `Removed { id }`），携带相同的 `seq`。受影响的价位从事件推导（与 L2 一样），但价位内的队列顺序与订单簿核对（`OrderBook::level_orders`）：增量是旧价位列表与新列表的差异，按 `Removed` → `Reduced` → `Added` 排序。因此冰山补充和 amend 时的丧失优先级表现为 remove-then-add（重排到队尾），并精确复现订单簿的 FIFO。`L3Feed::from_book` 从 `l3_snapshot` 以相同的 `seq` 播种该馈送。冰山隐藏储备不进入增量。
- **通道的端到端序列编号。** 所有 market data 通道通过统一的 `command_seq` 锚定到命令的 `seq`：快照携带 `current_seq()`，增量 `L2Update` / `L3Update` 和成交记录 `TapeTrade` 携带同一条命令的 `seq`。对一条命令而言所有通道携带同一个 `seq`，因此消费者应用 `seq` 大于快照值的增量，并通过共同锚点将各通道相互对齐。
- **成交带 (trade tape)。** `TradeTape::apply(&events)` 从事件流中提取公开匿名化的成交记录 `TapeTrade { seq, price, qty, taker_side }`——每个 `Event::Trade` 对应一条，按执行顺序；返回该命令的成交记录并将其累积在有上限的环形历史中（`new()` 无上限，`bounded(cap)` 保留最近 `cap` 条；读取用 `recent()` / `last()` / `len()` / `is_empty()`）。与 L2/L3 投影不同，成交带是**无需订单簿 (book-free)** 的——`apply` 只接收 `&[Event]` 而不读取订单簿，因此只看到事件流的消费者也能维护它。成交记录携带命令的 `seq`——与订单簿通道共同的锚点。匿名性针对所有者和标识符（订单 id 不进入成交带）；成交量则相反，是公开的并**完整**打印：遍历冰山会为每个撮合层产生一条成交记录，包括来自隐藏储备的量（订单簿隐藏*在簿*储备，成交带打印*已成交*量）。

## 持久化（日志、检查点与重放）

- `PersistentClob` 包装 `Clob` 并维护输入命令的预写日志：命令在返回事件**之前**被序列化并刷新到磁盘（`fsync`）——已确认的订单可在进程崩溃后存续。
- 恢复（`PersistentClob::open`）即**重放**：日志被重新喂入一个全新的 `Clob`；得益于确定性，订单簿、`order_id` 和 `seq` 被精确复现。被 gateway 拒绝的命令同样被记入日志（它们会消耗 `seq`）。
- 日志格式为自有二进制（仅 `std`）：段头 `magic "CLBW"` + 版本，带 varint 字段的命令记录，块以长度 + CRC32 分帧。组提交为 `submit_batch`（每批一次 `fsync`）。被中断的最后一条记录按 CRC 截断。
- **检查点。** `checkpoint()` 写入完整确定性状态的快照（序列器的计数器 `seq`/`next_order_id`、最后成交价、按优先级顺序排列的在簿订单连同冰山隐藏储备、休眠止损单、账户的净持仓）并轮转日志，只保留尾部。`open` 加载快照并**只重放快照之后的日志命令**——恢复不重放整个流。快照格式为 `magic "CLBS"` + 版本，varint 字段，带 CRC32 的通用帧（相同编解码器）。写入是原子的：临时文件 → `fsync` → `rename`。
- **检查点的崩溃安全。** 快照与日志的协调按 `seq` 幂等：恢复时只应用 `seq` 大于快照所覆盖范围的命令。因此在中途被打断的检查点（快照已写入，日志尚未轮转）既不会导致重复应用，也不会丢失命令。损坏的快照（CRC 不符）→ `JournalError::CorruptSnapshot`。
- 底层访问：`Journal`（写入器）、`read_commands`（命令流读取器）。

## 已知限制

- **止损按最后成交价触发**（而非按报价）；休眠止损单不反映在深度/`len()` 中；休眠止损单的 amend 不支持。
- **Iceberg**：隐藏储备不出现在 `depth()` / `len()` / `available_qty()` 中，也不计入 `Fok` 的流动性检查；可见部分的补充不产生事件；冰山的 `Modify` 总是重新排放订单（丧失优先级）。
- **Self-trade prevention** 仅在 `New` 上检查：`Modify`（amend）在不带 STP 的情况下重新撮合；`Fok` / post-only 的预检查将对手量整体计入，不减去所有者自身的流动性。`owner` 为私有——不进入 L2/L3 快照、增量和成交带。`decrement` 策略（在重叠处削减两者）暂未实现。
- **持久化——日志 + 检查点 + 重放**（`PersistentClob`）：快照和日志轮转只由显式的 `checkpoint()` 触发（没有基于计时器/容量的自动检查点——这由应用决定，核心保持无时钟）；逐命令 `fsync` 较慢（为吞吐量用组提交 `submit_batch`）；流中途的 `fsync` 失败应视为致命（重启并从日志恢复）。
- **Market data**：L2/L3 快照、L2（`L2Feed`）和 L3（`L3Feed`——带队列位置的按订单 market-by-order）增量、以及成交带（`TradeTape`）；所有通道锚定到同一条命令的 `seq`。订单簿馈送是同址的（与引擎同处）——`apply` 读取订单簿；成交带相反，是**无需订单簿 (book-free)** 的——消费者仅凭一条事件流维护它。（v0.4 阶段已完成。）
- **Pre-trade risk**（可选启用 `RiskConfig`）：tick/lot、以 mid 为基准的价格带、持仓限额（最坏情况净额）。休眠止损单在触发前不计入敞口量；`RiskConfig` 不持久化——应用在打开时指定它（重放时用相同的）。
- **手续费** maker/taker（可选启用 `FeeConfig`）：费率为名义金额的 ppm，带符号（maker 返佣）。在每笔成交上计算并输出到 `Event::Trade`（`taker_fee` / `maker_fee`）；核心**不累计**它们，`FeeConfig` **不持久化**。取整为向零截断。
- **逻辑时间**：`timestamp` 等同于 `seq`（为确定性不引入独立时钟）。

## 如何验证

```sh
cargo test
cargo run --example basic
cargo run --example replay
cargo run --release --example throughput
```
