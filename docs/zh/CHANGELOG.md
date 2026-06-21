# 更新日志

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
本项目遵循[语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 新增

- **maker/taker 手续费（v0.5，第 3 阶段）。** 新增模块 `src/fees.rs` 与公共类型 `FeeConfig`，
  附带构建器 `with_maker_ppm(rate)` / `with_taker_ppm(rate)`。费率以 **ppm**（**名义金额** `price·qty`
  的百万分之比例）表示且为**带符号**（`i64`）：正费率表示归交易所的手续费，**负的挂单方费率即返佣**
  （rebate，向挂单方支付）。通过 `Clob::with_fees(cfg)` / `Clob::with_risk_and_fees(risk, cfg)` 以及
  `PersistentClob::open_with_fees(path, cfg)` / `open_with_risk_and_fees(path, risk, cfg)` 接入；
  `Clob::new()` / `default()` 及原有构造函数保持**无手续费**（默认关闭，费率为 `0`），
  因此现有行为不变。手续费在**每一笔成交**于其产生处计算（引擎中、`on_trade` 回调里，此处两个参与方都可见），
  并通过 `Event::Trade` 的两个新字段输出——`taker_fee: i64` 与 `maker_fee: i64`。公式为
  `fee = price·qty·rate_ppm / 1_000_000`，中间过程在 `i128` 中以饱和方式计算，**向零截断取整**
  （对手续费与返佣对称）。完整成交量均被计费，包括冰山订单的隐藏储备（每个被撮合的层级都产生一条成交记录）。
  依据「events out, not state in」决策，手续费核心**不做累加**。
  因此**快照不变**（`CLBS` 仍为版本 `3`），日志（`CLBW`）不受影响，且 `FeeConfig` 与 `RiskConfig` 一样
  **不持久化**——由应用在打开时给定（重放时需要**相同**的配置；不过手续费不影响订单簿状态，仅影响输出的数值）。
  手续费**不会泄漏**到匿名化的 market data 通道——`taker_fee` / `maker_fee`
  不会进入 L2/L3 快照、增量与成交带（`TapeTrade`）。新增公共类型——`FeeConfig`。
  确定性核心未被破坏，无新增依赖（仅 `std`）。**本项收尾 v0.5 阶段。**
- **Gateway 中的交易前风控（v0.5，第 2 阶段）。** 新增模块 `src/risk.rs` 与公共类型
  `RiskConfig`，附带构建器 `with_tick(tick_size)` / `with_lot(lot_size)` / `with_price_band(band_ticks)` /
  `with_position_limit(limit)`。通过 `Clob::with_risk(cfg)` 与 `PersistentClob::open_with_risk(path, cfg)` 接入；
  `Clob::new()` / `default()` 与 `PersistentClob::open` 保持**无检查**，因此现有
  行为不变（风控严格为可选启用，默认关闭）。Gateway 变为有状态（持有
  `RiskConfig`），但公共的 `Gateway::validate(&Command)` 仍只做结构性检查；
  风控路径为内部路径。检查项：
  - **tick/lot。** 价格为 `tick_size` 的整数倍（限价、止损的 `trigger`、stop-limit 的两个字段、冰山订单的价格），
    冰山订单的数量与 `display` 为 `lot_size` 的整数倍。取值 `0`/`1` 关闭对应检查。
    市价订单跳过价格检查（没有价格），但 lot 仍应用于 `qty`。
  - **价格带。** 若价格距 **mid = (best_bid + best_ask) / 2** 超过 `band_ticks` 个 tick，则订单被拒绝：
    窗口为 `[mid − band_ticks·tick, mid + band_ticks·tick]`（tick=`max(tick_size, 1)`）。动态参考价取自
    订单簿（`OrderBook::mid()`），但 Gateway 自身不读取它——`Clob` 将 mid 注入校验上下文
    （阶段隔离得以保持）。在**冷启动/单边订单簿**（mid 未定义）时跳过价格带检查。
    价格带仅应用于 Limit/Iceberg；止损单（位于市场之外）与市价订单——跳过。
  - **持仓限额（最坏情况，带符号净额）。** 对于 `owner != 0`，在最坏情况下检查 `|net| ≤ limit`：
    买入时 `net + 买方敞口量 + qty ≤ limit`，卖出时 `−net + 卖方敞口量 + qty ≤ limit`。
    匿名（`owner == 0`）免于检查。在 `Modify` 上，先扣除该订单自身对敞口量的当前贡献，
    再加入新的 `qty`。敞口量计入冰山订单的完整剩余量（可见 + 隐藏储备）。
  净持仓由**订单簿本身**核算（单一事实来源）：`match_against` 可见每笔成交的双方参与者，
  因此净持仓（`i128`）与各方敞口量在订单簿内部于 `insert` / `cancel` /
  `reduce` / 撮合 / STP-`detach` 时更新——无需通过匿名化事件传递 `owner`。`Clob`/引擎仅
  **读取**这些聚合值用于校验。净持仓**经受**快照与重放：快照已版本化
  （`CLBS` → 版本 `3`，净持仓区段，账户按 `AccountId` 排序）；日志（`CLBW`）不变——
  命令相同，净持仓通过重新灌入挂单与重放成交而恢复。`RiskConfig` **不写入**日志/快照
  ——由应用在打开时给定；重放时需要提供**相同**的配置，否则历史会出现分歧。
  新增公共类型——`RiskConfig`；新增 `RejectReason`——`TickSize`、`LotSize`、`PriceBand`、`PositionLimit`。
  重构：`src/book.rs`（曾处于 400 行上限）被拆分为目录模块 `src/book/`——
  `mod.rs`（订单簿 + 索引 + 账户索引）、`matching.rs`（`match_against` / `detach`）、`accounts.rs`（`AccountBook`：
  净持仓 + 敞口量）。确定性核心未被破坏（`i128` 净持仓与 `u64` 敞口量，`HashMap` 迭代顺序不影响输出
  ——快照按账户排序），无新增依赖（仅 `std`）。已知边界：休眠止损单在触发前不计入敞口量；
  `RiskConfig` 在重放时由应用给定。本项收尾 v0.5 第二点；
  剩余 maker/taker **手续费**。
- **订单账户/所有者与自成交防范（STP）（v0.5，第 1 阶段）。** 订单获得了
  可选字段 `owner: AccountId`（`src/types.rs` 中新增的 `u64` 原语；`0` 表示匿名 /
  未设定）与 `stp: StpMode`（`Off` | `CancelTaker` | `CancelMaker` | `CancelBoth`），由
  构建器 `NewOrder::with_owner(id)` / `with_stp(mode)` 设定；默认构造函数置为
  `owner = 0`、`stp = Off`，因此现有行为不变（STP 严格为可选启用）。
  **自成交防范**在撮合时刻触发，按每个对手挂单做市方判断：当
  `taker.stp != Off && taker.owner != 0 && taker.owner == maker.owner` 时，不进行成交而是应用
  **主动方**策略（taker governs）——不读取挂单做市方的 `stp`，只需其所有者匹配即可。
  `CancelTaker` 取消主动方的剩余量（`Canceled`），挂单做市方保留；
  `CancelMaker` 取消遇到的那个挂单（`Canceled`），主动方沿队列继续前进；
  `CancelBoth` 取消两者。决策在遍历价位时**逐个**对每个挂单做市方作出，因此
  他人的流动性正常撮合，而自成交被精准化解。STP 同样应用于
  **已触发的止损单**（它携带自己的 `owner` / `stp`，并在激活时作为主动方行事）。
  `owner` 内联存储于 `RestingOrder`（对撮合循环而言缓存局部）并**经受**
  快照与重放：日志与快照已版本化（`CLBW` / `CLBS` → 版本 `2`）；`New` 记录
  以变体携带 `owner`，并在标志字节的空闲位中携带 `stp`。market data 投影保持
  **匿名化**——`owner` 不会进入 L2 / L3 / 成交带。新增公共类型——
  `AccountId`、`StpMode`、`MatchOutcome`。确定性核心未被破坏（新的 `u64`
  仅作相等性比较，`HashMap` 迭代顺序不影响输出），无新增依赖
  （仅 `std`）。已知边界：STP 在 `New` 上检查（不在 `Modify` 上——amend 会无 STP
  地重新撮合）；`Fok` / post-only 的预检查将对手量整体计入，不扣除自身的
  流动性。本项收尾 v0.5 第一点；交易前风控与手续费——接下来进行。
- **Market data：增量 L3 更新与跨通道端到端序列编号（v0.4，第 4 阶段——收尾 v0.4）。**
  新增模块 `src/l3feed.rs` 与公共类型 `L3Delta`（`Added { id, side, price, qty }` /
  `Reduced { id, qty }` / `Removed { id }`）、`L3Update { seq, deltas }` 与 `L3Feed`——增量
  **按订单（market-by-order）**通道，带队列位置。`L3Feed::apply(&events, book)` 将一条命令的事件
  转换为按订单的增量：受影响的价位从事件推断（与 `L2Feed` 一样），
  而每个价位内部的队列顺序通过新的只读访问器
  `OrderBook::level_orders(side, price)` 与订单簿核对。Survivors（保住队列位置的订单）给出 `Reduced`，
  退出者给出 `Removed`，新增/被移到队尾者给出 `Added`；增量按
  remove → reduce → add 的顺序发出，因此**冰山补充**与**amend 时的丧失优先级**（移到
  价位队尾）被正确表达为 remove-then-add，并在消费者端精确复现订单簿的 FIFO。
  `L3Feed::new()` 从空订单簿启动，`L3Feed::from_book(book)`——以相同 `seq` 从快照播种。
  投影保持**公开且匿名化**：仅给出冰山订单的可见 peak，隐藏储备不会
  进入增量。
  **data 通道的端到端序列编号。** 所有 market data 通道均通过统一的
  `command_seq` 锚定在命令的 `seq` 上（单一事实来源，取代分散的输出）：`l2_snapshot` / `l3_snapshot` 快照
  携带 `Clob::current_seq()`，`L2Update` / `L3Update` 增量与 `TapeTrade` 成交记录携带同一个命令
  `seq`。对于同一条命令，所有通道携带**同一个** `seq`，因此消费者应用
  `seq` 大于快照 `seq` 的增量，并通过共同锚点将各通道相互对齐。新增公共类型——`L3Delta`、
  `L3Feed`、`L3Update`；新增只读访问器 `OrderBook::level_orders`。确定性核心未
  改动，无新增依赖（仅 `std`）。随此步骤 **v0.4（Market data）收尾**。
- **Market data：成交带（trade tape）（v0.4，第 3 阶段）。** 新增模块 `src/tape.rs` 与公共
  类型 `TapeTrade`（匿名化成交记录：`seq` + `price` + `qty` + `taker_side`）与 `TradeTape`——
  事件流之上的独立 data 通道。`TradeTape::apply(&events)` 为每个 `Event::Trade` 提取一条成交记录
  （按执行顺序——扫过多个价位时 best price first），
  返回该命令的成交记录并将其累积进有界的环形历史：`new()`——无上限，
  `bounded(cap)`——仅保留最近 `cap` 条；读取——`recent()`（从旧到新）、`last()`、`len()`、
  `is_empty()`。与 L2/L3 投影不同，本通道**无需订单簿（book-free）**：`apply` 仅取 `&[Event]` 而不
  访问订单簿——成交带可由仅见事件流的消费者维护。每条成交记录携带
  命令的 `seq`——与订单簿通道（`L2Update` 与快照）的共同锚点。匿名化针对所有者与
  标识符（订单 id 不会进入成交带）；成交量是公开的并完整打印，
  因此遍历冰山订单时每个被撮合的层级都产生一条成交记录，包括来自隐藏储备的成交量
  （订单簿隐藏*挂着的*储备，成交带打印*已成交的*）。确定性核心未改动，
  无新增依赖（仅 `std`）。新增公共类型——`TapeTrade`、`TradeTape`。剩余的
  v0.4 步骤（L3 增量与跨通道端到端序列编号）由下一阶段收尾。
- **Market data：增量 L2 更新（v0.4，第 2 阶段）。** 新增公共类型
  `L2Update`（稀疏帧：`seq` + 由 `L2Level` 组成的 `bids` / `asks`，`qty == 0` 的价位
  表示删除，每一侧价格 best-first）与 `L2Feed`——有状态的只读投影，
  将一条命令的 `Vec<Event>` 转换为**仅变化的**价位的增量。
  `L2Feed::apply(&events, book)` 从事件推断受影响的价位（`Trade` 的挂单做市方一侧；
  内部镜像 `order_id → (side, price)` 用于 `Resting` / `Modified` / `Canceled` / `Filled`
  这些不携带价格的事件），并将每个价位的新聚合值与订单簿核对——因此即使在
  事件流「沉默」之处，增量也精确：在**冰山补充**（无事件，但补充始终发生在
  成交价上，而该价已在受影响之列）与 **modify 重定价**（旧价位在
  `Resting` 将镜像更新为新价之前由 `Modified` 加入）时。`L2Feed::new()` 从
  空订单簿启动，`L2Feed::from_book(book)`——从当前状态播种（与 `l2_snapshot` 配对，
  使用相同 `seq`）：快照给定基线，增量在不重新发送整个
  订单簿的情况下保持其同步。帧携带命令的 `seq`——与快照的共同锚点。投影保持**公开且
  匿名化**：冰山订单的隐藏储备不会进入增量。新增一个只读访问器
  `OrderBook::level_qty(side, price)`；确定性核心未改动，无新增依赖
  （仅 `std`）。新增公共类型——`L2Update`、`L2Feed`。L3 增量、成交带与
  跨通道的序列号一致性——v0.4 剩余步骤。
- **Market data：L2 与 L3 订单簿快照（v0.4，第 1 阶段）。** `Clob` 上的两个新方法：
  `l2_snapshot(depth)`——**按价格聚合（market-by-price）**，按价位聚合的深度
  （`L2Snapshot`，其 `bids` / `asks` 由 `L2Level { price, qty }` 组成，最优价在前，每一侧不超过
  `depth` 个价位），以及 `l3_snapshot()`——**按订单（market-by-order）**，按单个
  订单列出的订单簿（`L3Snapshot`，由 `L3Order { id, side, price, qty }` 组成，价格 best-first 且价位内 FIFO
  ——可见队列位置）。两个快照均以当前 `seq`（`current_seq`）打标签，以便
  消费者能相对于未来的增量更新对其排序。投影
  **公开且匿名化**：对冰山订单仅给出可见 peak——隐藏储备
  **不会进入** L2/L3。L2 是 L3 按价格的聚合（该不变量由测试验证）。在新模块
  `src/marketdata.rs` 中实现为只读投影，建于公共的 `book()` / `current_seq()` 之上：
  确定性核心未改动，无新增依赖（仅 `std`）。新增公共类型——
  `L2Level`、`L2Snapshot`、`L3Order`、`L3Snapshot`。增量更新、成交带与
  跨通道的序列号一致性——v0.4 接下来的步骤。
- **持久化：订单簿快照（v0.3，第 2 阶段）。** 新方法
  `PersistentClob::checkpoint()` 将完整确定性状态的**快照**写入磁盘
  （序列器的 `seq` 与 `next_order_id`、最近一次成交价、所有挂单
  按优先级顺序连同冰山订单的隐藏储备、休眠止损单），随后
  将日志截断至尾部（以新的 `base_seq` 进行段轮转）。恢复
  （`PersistentClob::open`）现在加载快照并**仅重放尾部**日志，而
  非从零开始重放整个流。快照格式为自有二进制、仅 `std`：头部
  `magic "CLBS"` + 版本、varint 字段、带 CRC32 的通用帧（与日志相同的编解码器）。
  快照写入是原子的（写入临时文件 → `fsync` → `rename`）。快照
  与日志的协调按 `seq` 幂等：恢复时仅应用 `seq` 大于
  快照覆盖范围的日志命令，因此中途被打断的检查点（快照
  已写入、日志尚未轮转）既不会导致重复应用，也不会丢失命令。
  损坏的快照（CRC 不匹配）被拒绝为 `JournalError::CorruptSnapshot`——此时日志
  仍是事实来源。快照与日志存放在一起（`<journal>.snap`），
  公共 API 增加了一个方法 `checkpoint`；无新增依赖（仅 `std`），
  确定性核心未改动。新增错误变体——`JournalError::CorruptSnapshot`。
- **持久化：命令日志与重放（v0.3，第 1 阶段）。** 新封装
  `PersistentClob` 包裹 `Clob` 并维护输入命令的预写日志：每条
  命令在返回事件**之前**被序列化并刷盘（`fsync`），因此
  已确认的订单可经受进程崩溃。打开时（`PersistentClob::open`）
  状态通过**重放**恢复——记录的命令流被重新送入一个全新的
  `Clob`；得益于确定性，订单簿、`order_id` 与 `seq` 被精确复现（被
  gateway 拒绝的命令同样被记入日志——它们消耗 `seq`）。日志格式为自有
  二进制、仅 `std`：段头 `magic "CLBW"` + 格式版本、带 varint 字段的命令记录、
  数据块以长度 + CRC32 分帧（组提交——`submit_batch`）。
  在恢复时，半写状态被打断的最后一条记录按 CRC 被截除。
  低层访问：`Journal`（写入器）、`read_commands`（读取器）。新增公共
  类型——`PersistentClob`、`Journal`、`JournalError`、`CodecError`，以及函数 `read_commands`。
  确定性核心未改动：持久化是 `Clob` 之上的一层，无时钟无线程；
  无新增依赖（仅 `std`）。用于加速恢复的订单簿快照——接下来的
  步骤（第 2 阶段）。
- **冰山订单（`OrderType::Iceberg { display }`）**——带可见部分（peak，
  `display`）与隐藏储备的订单。构造函数 `NewOrder::iceberg(side, price, qty, display)`，
  其中 `qty` 是完整数量，`display` 是可见部分的大小。在订单簿、`depth()`、
  `len()`、`total_qty` 与 `available_qty()` 中**只计入可见 peak**；储备
  存储于订单簿的旁路索引中，不予显示。当可见 peak 被完全
  撮合时，从储备补充出新的 peak `min(display, hidden)` 并放置于其
  价位的**队尾**——在已显示的订单面前丧失时间优先级（冰山订单的
  标准规则）。足够大的吃单方会单次「扫过」整个冰山订单（包括隐藏部分），逐层进行。
  补充**不产生事件**（体现在 `depth()` 中）；增量更新是
  v0.4 的单独条目。作为吃单方，冰山订单按完整 `qty` 撮合，如同普通限价订单。Gateway 拒绝 `display == 0`
  （`ZeroQuantity`）与 `price == 0`（`InvalidPrice`）；`display >= qty` 退化为普通的
  完全可见订单。对于 `Fok`，隐藏储备**不计入**流动性
  检查（只计已显示的数量）。冰山订单的 `Modify` 将 `qty` 视为
  新的完整数量，保留 `display` 并总是重新放置订单（丧失
  优先级）。未新增 `Event` / `RejectReason`。
- **止损单与 stop-limit（`OrderType::Stop` / `OrderType::StopLimit`）**——带触发价的延迟订单。构造函数 `NewOrder::stop(side, trigger, qty)`（触发时行为如市价订单）与 `NewOrder::stop_limit(side, trigger, limit, qty)`（触发时——按 `limit` 价的限价订单）。Gateway 拒绝 `trigger == 0` 的止损单以及 `trigger == 0` 或 `price == 0` 的 stop-limit，作为 `InvalidPrice`。触发绑定于**最近一次成交价**：buy-stop 在 `last >= trigger` 时激活，sell-stop 在 `last <= trigger` 时激活。触发前订单「休眠」于独立的止损单簿中，且在订单簿的 `depth()` / `len()` 中**不可见**；若市场已越过触发价，止损单在接收时立即触发。激活是**级联**的：已触发订单的成交推动价格，并可能激活下一批止损单（激活顺序——按接收时间，确定性的）。`Cancel` 取消休眠止损单；休眠止损单的 `Modify` 暂不支持（被拒绝为 `UnknownOrder`）。
- **`Triggered` 事件**——止损单已触发并正在进行撮合；随后是常规的结果事件（`Trade` / `Resting` / `Filled` / `Canceled`），如同 `New` 在 `Accepted` 之后。激活事件以当前命令的 `seq` 打标签，订单身份——按 `order_id`。
- **`Clob::pending_stops()`**——休眠（尚未触发）止损单的数量。
- **Post-only（`TimeInForce::PostOnly`）**——「仅挂单方」订单：若限价会立即穿越价差，则在 `Accepted` 之前被拒绝（`RejectReason::WouldCross`）且不进入订单簿；否则行为如 `Gtc` 并挂入订单簿。检查在引擎中、接收之前进行（如同 `Fok` 的预检查）；对 `Market` 始终视为穿越。
- **`Modify` 命令（amend）**——修改挂单的价格和/或数量（`Command::Modify`、`ModifyOrder::new`）。同价下减少数量保留时间优先级（原地修改，`O(1)`）；改价或增加数量丧失优先级——订单被取消并以相同 `order_id` 重新进行撮合，穿越价差时被执行。
- **`Modified` 事件**——`Modify` 接收的确认；随后是结果事件（`Resting` / `Trade` / `Filled`），如同 `New` 在 `Accepted` 之后。

### 变更

- **Order book**——带侵入式 FIFO 列表的 slab 内存池（`Node`、`Slab`、`PriceLevel` 以及操作 `link_back` / `unlink`）已从 `src/book.rs` 移出至新模块 `src/slab.rs`（按职责拆分：`book.rs` 保持在 400 行上限以内）。行为与复杂度未改变。
- **Order book**——价位内部使用侵入式双向 FIFO 链表替代 `VecDeque`；订单节点存储于公共内存池（slab）中，附带空闲槽列表。价位内的取消现在为 `O(1)` 而非 `O(n)`。
- **Matching Engine**——移除了用于已成交订单标识符的 `Vec` 分配：挂单做市方直接在撮合循环中从索引移除，节点槽归还内存池。
- **Matching Engine**——通用撮合路径（TIF 预检查、撮合、剩余量挂入/取消）被提取至 `settle` / `precheck` 并被新订单、amend 与止损单激活复用；新增了止损单簿与按最近成交价的级联激活。产生成交的 `Modify` 现在也可激活止损单。

其余计划项见 [ROADMAP.md](ROADMAP.md)。

## [0.1.0] — 2026-06-20

首个发布。带流水线 `Gateway → Sequencer → Matching Engine → Output` 的确定性 CLOB 核心。

### 新增

- **`Clob` 流水线**——单一入口 `submit()` / `submit_into()`，串联所有阶段。
- **Gateway**——订单校验：零数量（`ZeroQuantity`）与限价订单零价格（`InvalidPrice`）时拒绝。
- **Sequencer**——单调递增的序列号与订单标识符，用于确定性顺序。
- **Matching Engine**——按价格-时间优先（价位内 FIFO）撮合。
- **Order book**——每一侧按价位组织的 `BTreeMap`、用于价位内 FIFO 的 `VecDeque`、用于取消的 `HashMap` 订单索引。
- **订单类型**——`Limit`、`Market`。
- **Time-in-force**——`Gtc`、`Ioc`、`Fok`。
- **事件**——`Accepted`、`Trade`、`Resting`、`Filled`、`Canceled`、`Rejected`。
- **订单簿访问**——`best_bid`、`best_ask`、`spread`、`depth`、`len`、`contains`、`available_qty`。
- **示例**——`examples/basic.rs`（演示事件与订单簿）、`examples/throughput.rs`（压测运行）。
- **测试**——12 个集成测试，涵盖撮合、优先级、TIF、取消与订单簿不变量。
- **文档**——`docs/` 目录（状态、架构、更新日志、roadmap）。

### 实现要点

- 整数价格与数量（`u64`），无浮点数。
- 核心无外部依赖（仅 `std`）。
- Rust 2024 版次；`release` 配置带 LTO 与 `panic = "abort"`。

[Unreleased]: https://example.com/clob/compare/v0.1.0...HEAD
[0.1.0]: https://example.com/clob/releases/tag/v0.1.0
