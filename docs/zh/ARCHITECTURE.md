# 架构

## 概述

`clob` 是一个确定性的中央限价订单簿引擎，组织为一条由四个阶段构成的流水线。每条命令按顺序经过各个阶段，并产生一个事件向量。

```
                    ┌──────────┐   ┌────────────┐   ┌──────────────────┐   ┌──────────┐
   Command  ─────▶  │ Gateway  │─▶ │ Sequencer  │─▶ │ Matching Engine  │─▶ │  Output  │ ─────▶ Vec<Event>
(New/Cancel/Modify) └──────────┘   └────────────┘   └──────────────────┘   └──────────┘
                    校验与          分配               订单簿 +              事件：
                    归一化          seq 和 order_id    price-time 撮合       Trade / Resting / ...
```

连接各阶段的是类型 [`Clob`](../../src/clob.rs)。公共 API 极简：

```rust
let mut clob = Clob::new();
let events: Vec<Event> = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 5)));
```

## 原则

1. **确定性。** 单个合约由单个线程按序列号的严格顺序处理。相同输入 → 相同输出，永远如此。这带来了可重现性、审计、副本以及（未来的）按日志恢复。
2. **整数运算。** 价格以 tick（最小报价单位）为单位，数量以 lot（最小数量单位）为单位（`u64`）。不使用任何 `f64`：货币计算必须精确且可重复。
3. **输出事件，而非输入变更。** 引擎不对外暴露可变状态——它返回一个事件流，对行情数据 (market data) 和日志而言都很自然。
4. **单个订单簿内部不并行**——在订单簿内部使用线程会破坏确定性。

## 流水线阶段

### Gateway — [src/gateway.rs](../../src/gateway.rs)

第一道屏障。在命令抵达引擎之前校验其正确性：

- `qty == 0` → `RejectReason::ZeroQuantity`；
- 价格为零 → `RejectReason::InvalidPrice`：限价订单 `price == 0`、止损单 `trigger == 0`、止损限价单 `trigger == 0` 或 `price == 0`；
- `Modify` 的 `qty == 0` 或 `price == 0` 按相同规则被拒绝；
- 撤单总是通过（订单是否存在由引擎检查）。

**交易前风控 (opt-in)。** 当配置了 `RiskConfig`（[src/risk.rs](../../src/risk.rs)；通过 `Clob::with_risk` / `PersistentClob::open_with_risk` 接入，默认关闭，行为不变）时，Gateway 额外检查：

- **tick/lot**——价格相对 `tick_size` 的整除性（限价价格、`trigger`、止损限价单的两个字段、冰山订单的价格）以及数量/`display` 相对 `lot_size` 的整除性；
- **价格带**——价格处于相对 `mid = (best_bid + best_ask) / 2` 的 `±band_ticks` 个 tick 的窗口内（仅 Limit/Iceberg；止损单和 market 略过；在冷启动/单边订单簿上跳过）。mid 由 `Clob` 从订单簿计算并放入校验上下文中——Gateway 自身不读订单簿（保持阶段分离）；
- **持仓限额**——带符号净额的最坏情况 `net + 该侧敞口量 + qty ≤ limit`（仅 `owner != 0`；匿名者豁免）。

拒绝原因为 `TickSize`、`LotSize`、`PriceBand`、`PositionLimit`。配置是静态且不可变的，因此确定性得以保持；按账户的净持仓和敞口量由订单簿维护（见下）。

### Sequencer — [src/sequencer.rs](../../src/sequencer.rs)

为每个事件分配一个单调递增的 `seq`，并为每个新订单分配一个唯一的 `order_id`（从 1 开始）。这是确定性的心脏：顺序在此固定，此后不再改变。`seq` 同时用作逻辑时间戳。

### Matching Engine — [src/engine.rs](../../src/engine.rs)

将经过校验和编号的命令应用到订单簿，并产生事件。负责订单类型和 time-in-force 的语义：

- **FOK**——预检查 `available_qty`；如果无法完全成交，订单被拒绝（`InsufficientLiquidity`）且不进入订单簿。
- **Post-only**——预检查 `would_cross`；如果限价订单会立即穿越价差，订单在 `Accepted` 之前被拒绝（`WouldCross`）且不进入订单簿（`Market` 总是被视为穿越）。否则其行为如同 `Gtc`——进入订单簿。
- 否则——`Accepted`，然后与对手方撮合（`Trade` 事件）。
- 撮合之后：
  - 剩余量 `0` → `Filled`；
  - 剩余量 `>0` 且为 `Limit` / `Iceberg` +（`Gtc` / `PostOnly`）→ 进入订单簿（`Resting`）；对于 `Iceberg`，订单簿中只显示可见的 peak，其余进入隐藏储备；
  - 否则（`Market`、`Ioc`、未通过的 `Fok`）→ 剩余量被撤销（`Canceled`）。

撤单：`execute_cancel` 将订单从订单簿移除（`Canceled`），或拒绝不存在的订单（`UnknownOrder`）。

修改 (amend)：`execute_modify` 找到挂着的订单（否则 `UnknownOrder`）并发出 `Modified`。在相同价格下减少数量——`O(1)` 的原地修改并保留优先级（随后是带新数量的 `Resting`）。改变价格或增加数量——丧失优先级：订单被取下，并以相同的 `order_id` 重新经过撮合（若穿越价差——`Trade`，然后 `Filled` 或 `Resting`）。这样，amend 之后订单簿仍保持未交叉。

止损单（`Stop` / `StopLimit`）不会立即进入主订单簿：引擎将它们「停泊」在单独的止损单簿（[src/stops.rs](../../src/stops.rs)）中并发出 `Accepted`。触发与**最近成交价**绑定：buy-stop 在 `last >= trigger` 时激活，sell-stop 在 `last <= trigger` 时激活。在任何产生成交的命令之后（以及在接收止损单后立即），引擎会跑一遍已触发的止损单：对每个——`Triggered`，然后是常规撮合（`Stop` 当作市价订单，`StopLimit` 当作具有相同 TIF 的限价订单），经由与 `New` 相同的 `settle`/`precheck` 路径。已激活订单的成交会推动最近成交价，因此激活是**级联**的：一个止损单可以拉动下一个。级联是确定性的（激活顺序按接收时间）且有限的（休眠止损单的集合严格递减）。激活事件以当前命令的 `seq` 标记，订单通过 `order_id` 识别。`Cancel` 取下休眠止损单；休眠止损单的 `Modify` 目前被拒绝（`UnknownOrder`）。

冰山订单（`Iceberg { display }`）像普通限价订单一样进入订单簿，但只显示可见部分（peak，`display`）；剩余的数量躺在隐藏储备中（订单簿中的旁路索引），不在 `depth()` / `len()` / `available_qty()` 中可见。当可见的 peak 被完全撮合后，从储备中补充一个新的 peak `min(display, hidden)` 并放到其价位的**队尾**——位于所有已显示订单之后，也就是丧失时间优先级（冰山订单的标准规则）。一个足够大的对手单会在一次 `match_against` 调用中逐层穿过整个冰山订单。补充会改变订单簿状态，但不产生事件。作为吃单方 (taker)，冰山订单按完整的 `qty` 撮合（如同限价订单）；`Fok` 只看到显示的数量（隐藏储备不计入流动性检查）。冰山订单的 `Modify` 将 `qty` 解释为新的完整数量，保留 `display`，并重新放置订单（丧失优先级）。

**自成交防范 (STP)。** 订单可以携带所有者（`owner: AccountId`）和模式 `stp: StpMode`（`Off` | `CancelTaker` | `CancelMaker` | `CancelBoth`；默认 `Off`，`owner = 0`）。撮合时，引擎**对每个对手挂单做市方 (maker)** 检查自成交：如果 `taker.stp != Off`、`taker.owner != 0` 且做市方的所有者与主动方的所有者相同，则不进行成交，而是应用**主动方**的策略（由吃单方决定，taker governs——不读取挂着的做市方的 `stp`，它只需在所有者上匹配即可）：`CancelTaker` 中断遍历并撤销主动方的剩余量（`Canceled`），做市方完好；`CancelMaker` 撤销遇到的挂着的订单（`Canceled`），主动方继续遍历队列；`CancelBoth` 撤销两者。这个决定是局部的——路径上他方的流动性正常撮合，因此 STP 不会为其他参与者打乱 price-time priority。`owner` 是一个**不透明的相等性令牌**：核心只对其进行比较，内联存储于 `RestingOrder` 上，且不在公共行情数据投影中输出。触发的止损单携带自己的 `owner` / `stp`，并在激活时如同普通主动方一样服从 STP。STP 在 `New` 上检查；`Modify`（amend）在重新撮合时不带 STP。

**手续费（maker/taker）。** 当配置了 `FeeConfig`（[src/fees.rs](../../src/fees.rs)；构建器 `with_maker_ppm` / `with_taker_ppm`；默认关闭）时，引擎在**每笔成交**诞生的那一点——`settle` 内部的 `on_trade` 回调中——计算手续费，那里两个参与者（主动方和遇到的做市方）都在作用域内。费率以**名义金额 (notional) 的 ppm（百万分之）**给定，且**带符号**（`i64`）：`fee = price · qty · rate_ppm / 1_000_000`（中间用 `i128` 并饱和 (saturating)，向零截断取整）；正费率是归交易所的手续费，**负的做市方费率是返佣 (rebate)**（负的 `maker_fee`）。结果通过 `Event::Trade` 的两个字段（`taker_fee` / `maker_fee`）输出；全部已成交的数量都被计费，包括冰山订单的隐藏储备（对每个撮合的层产生一条成交记录 (print)）。按照「输出事件，而非内部状态」的原则，核心**不累计**手续费：快照不变，日志不受影响，`FeeConfig` 不持久化（与 `RiskConfig` 一样——由应用在打开时给定）。通过 `Clob::with_fees` / `with_risk_and_fees` 和 `PersistentClob::open_with_fees` / `open_with_risk_and_fees` 接入。

### Output — [src/output.rs](../../src/output.rs)

统一的事件枚举 `Event`：

| 事件 | 何时 |
| --- | --- |
| `Accepted` | 订单被引擎接受 |
| `Trade` | 发生了成交（吃单方 ↔ 做市方，价格，数量；`taker_fee` / `maker_fee`——手续费，带符号 `i64`） |
| `Resting` | 剩余量进入订单簿 |
| `Filled` | 订单被完全成交 |
| `Canceled` | 订单/剩余量被撤销 |
| `Modified` | 订单被修改（amend）；其后是结果事件 |
| `Triggered` | 止损单被触发；其后是结果事件 |
| `Rejected` | 拒绝（gateway、FOK 流动性不足或 post-only 穿越价差） |

## 订单簿结构 — [src/book/](../../src/book/)

该模块是目录 `src/book/`：`mod.rs`（订单簿的结构与操作、索引）、`matching.rs`（`match_against` / `detach`——撮合算法与自成交防范）、`accounts.rs`（`AccountBook`——用于交易前风控的按账户聚合）。子模块是 `book` 的子级，因此能看到 `OrderBook` 的私有字段；这种拆分由每文件 400 行的限制所决定。

```
                 OrderBook
   ┌──────────────────────────────────────┐
   │ bids:  BTreeMap<Price, PriceLevel>    │   最高价格 = 最佳 bid
   │ asks:  BTreeMap<Price, PriceLevel>    │   最低价格 = 最佳 ask
   │ slab:  Vec<Node> + 空闲列表            │   订单节点的内存池 (arena)
   │ index: HashMap<OrderId, {side, slot}>  │   为撤单 O(1) 查找节点
   │ reserves: HashMap<OrderId,{disp,hid}>  │   冰山订单的隐藏部分
   │ accounts: HashMap<AccountId,{net,open}>│   按账户的净持仓和敞口量
   └──────────────────────────────────────┘
                    │
                    ▼  PriceLevel（单个价位）
   ┌────────────────────────────────────┐
   │ head, tail: Option<u32>             │   侵入式双向 FIFO 链表
   │ total_qty: Qty                      │   该价位的总数量
   └────────────────────────────────────┘

   Node（在 slab 中）：{ order: RestingOrder, prev, next: Option<u32> }
```

- `bids` 和 `asks` 按价格排序（`BTreeMap`）：最佳价格位于树的端点。
- 价位内部，订单由侵入式双向链表连接（`head`/`tail` → slab 中的槽）：到达顺序就是时间优先级。在队尾插入，从队头撮合。
- 订单节点存活于共享的内存池 (arena)（`slab`）中并带有空闲槽列表——分配被复用，稳态下每个订单无需分配内存。slab、节点和 `PriceLevel`（侵入式链表及其操作 `link_back` / `unlink`）被移到 [src/slab.rs](../../src/slab.rs)。`RestingOrder` 额外携带 `owner`（用于 STP）——它不进入深度和公共投影。
- `index` 将 `OrderId → (side, slot)` 对应起来，因此撤单可在 `O(1)` 内将节点从链表中摘下，无需遍历价位。
- `reserves` 持有冰山订单的隐藏部分（`OrderId → {display, hidden}`）：可见的 peak 是 slab 中的普通节点并计入价位的 `total_qty`，而储备单独存放且不进入深度。只有带非空储备的冰山订单才会出现一条记录。
- `accounts`（[src/book/accounts.rs](../../src/book/accounts.rs)）维护用于交易前风控的按账户聚合：净持仓（`i128`，带符号）和按侧的敞口量（完整剩余量，含冰山订单的隐藏储备）。在 `insert` / `cancel` / `reduce` / 撮合 / STP-`detach` 时更新——订单簿是挂着数量的唯一变更者，并能看到每笔成交的两个参与者，因此聚合不会与订单簿失同步，也无需通过匿名化事件透传 `owner`；`owner == 0`（匿名）不计入。净持仓挺过快照，敞口量通过重新灌入挂着的订单恢复。

### 复杂度

| 操作 | 复杂度 | 备注 |
| --- | --- | --- |
| 最佳 bid/ask | `O(log L)` | `L`——价位数量 |
| 插入剩余量 | `O(log L)` | 查找/创建价位 |
| 撮合步 | 每价位 `O(log L)` + 每订单 `O(1)` | 从侵入式链表队头的 FIFO |
| 撤单 | `O(log L)` | 在树中查找价位 + 在 `O(1)` 内将节点从链表摘下 |

## 撮合算法

Price-time priority：最激进的价格最先成交，价格相同时——更早到达的订单。核心是 `OrderBook::match_against`：

```
match_against(taker_side, limit_price, qty):
    opp = 若 Buy 则 asks 否则 bids
    while qty > 0:
        best = 若 Buy 则 min(asks) 否则 max(bids)      # 最佳对手价格
        if best 不存在: break                           # 流动性耗尽
        if not crosses(limit_price, best): break        # 价格不再穿越
        level = opp[best]
        while qty > 0 且 level 非空:
            head = slab[level.head]                     # 最早的做市方
            if taker.stp != Off 且 head.owner == taker.owner:  # STP: 自成交
                应用吃单方策略 (cancel taker → break; maker → 撤销并 continue; both)
            traded = min(qty, head.qty)
            head.qty -= traded;  qty -= traded
            emit Trade(maker=head.id, price=best, qty=traded)
            if head.qty == 0:                           # 节点耗尽:
                index.remove(head.id)                   #   从索引移除,
                level.head = head.next; free(slot)      #   摘下队头, 归还槽
        if level 为空: 删除价位 best
    return qty                                          # 未成交的剩余量
```

对于冰山订单，`if head.qty == 0` 这一步不删除节点，而是从隐藏储备补充可见的 peak（`min(display, hidden)`）并将节点移到价位的**队尾**：隐藏部分重新排队，位于已显示订单之后。仅当储备耗尽时才删除节点。储备严格递减，因此补充是有限的。

穿越条件 `crosses`：

- market 订单（`limit_price = None`）——总是穿越；
- `Buy`——`limit_price >= best_ask`；
- `Sell`——`limit_price <= best_bid`。

同一谓词被提取到 `OrderBook::would_cross` 并由引擎中的 post-only 闸门使用：「仅做市方」订单若会立即撮合，则在接收前被拒绝。

成交按做市方价格（`best`）执行——即挂在订单簿中的那一方，依据 price-time 规则。被完全成交的做市方立即从 `index` 中移除，其槽归还到 slab 的空闲槽列表。

不变量：处理任何命令之后，订单簿**未交叉**——会穿越价差的对手单必然在到达时就已撮合（由测试 `book_is_never_crossed_after_matching` 验证）。

## 端到端示例

订单簿：ask `101 ×10`、ask `102 ×5`、bid `100 ×7`。
来了一个激进的 `Buy` 限价订单 `101 ×12`（`Gtc`）：

1. Gateway：通过。Sequencer：`seq=4`、`order_id=4`。→ `Accepted`。
2. 与 asks 撮合：最佳 ask `101` 穿越（`101 >= 101`）→ 成交 `101 ×10`。下一个 ask `102` 已不穿越（`101 < 102`）→ 停止。剩余量 `2`。
3. `Limit` + `Gtc`，剩余量 `>0` → 作为 bid `101 ×2` 进入订单簿。→ `Resting`。

订单簿结果：bid `101 ×2`、bid `100 ×7`、ask `102 ×5`、价差 `1`。
（正是这个场景由 `examples/basic.rs` 打印。）

## 行情数据 (market data)——快照、增量更新和成交带 (trade tape) — [src/marketdata.rs](../../src/marketdata.rs)

行情数据 (market data) 是订单簿面向外部观察者的**只读投影**，与私有事件流分离：引擎写订单簿，行情数据只读它。它实现为 `Clob` 上的两个方法，建立在公共的 `book()` / `current_seq()` 之上，不改动确定性核心：

- `l2_snapshot(depth)`——**按价格聚合 (market-by-price)**：按价位聚合的深度（`L2Snapshot`，其 `bids` / `asks` 由 `L2Level { price, qty }` 构成），最佳价格在前，每侧不超过 `depth` 个价位。来源是 `OrderBook::depth`。
- `l3_snapshot()`——**按订单 (market-by-order)**：按单个订单的订单簿（`L3Snapshot` 由 `L3Order { id, side, price, qty }` 构成），按价格 best-first 且价位内 FIFO——也就是队列位置可见。来源是 `OrderBook::resting_orders`（按价格的稳定排序保留价位内的队列顺序）。

两个快照都以当前 `seq` 标记，以便消费者能将它们相对未来的增量更新排序（v0.4 的下一步）。该投影是**公共且匿名化的**：对冰山订单只输出可见的 peak——隐藏储备不进入 L2/L3（隐私由参与者*放入*订单簿的内容决定，而非由订阅源的粒度决定）。L2 是 L3 按价格的聚合：`sum(每价位的 L3 qty) == L2 qty`（不变量由测试 `l2_equals_l3_aggregated` 验证）。

### 增量 L2 更新 — `L2Update` / `L2Feed`

快照是整个订单簿；为避免每次变化时都重新发送它，行情数据能够输出**增量 L2 增量 (delta)**。`L2Feed` 是有状态的只读投影：`apply(&events, book)` 将一条命令的 `Vec<Event>` 转换为 `L2Update { seq, bids, asks }`——一个**仅含已变化**价位的列表（`qty == 0` 的价位表示删除；价格在每侧 best-first）。该帧携带命令的 `seq`，与快照共用锚点：消费者在 `seq = N` 取 `l2_snapshot`（或通过 `L2Feed::from_book` 以相同状态播种订阅源）并应用后续增量，将本地订单簿保持同步而无需重新发送整个深度。

哪些价位受影响，订阅源**从事件中**推导，而不扫描订单簿：`Trade`——做市方一侧的价格（`taker_side.opposite()`）；`Resting`——进入订单簿的剩余量的侧和价格；`Canceled` / `Filled` / `Modified`——订单的先前位置，取自内部镜像 `order_id → (side, price)`（这些事件只携带 `order_id`）。每个受影响价位的新聚合值**从订单簿中读取**（`OrderBook::level_qty`），而非从事件累计，因此即便在两种由事件流不可见的情形下增量也是精确的：**冰山订单补充**（没有事件，但补充总是发生在成交价上，而成交价已在受影响价位集合中）和 **modify 重定价**（旧价位在 `Modified` 时加入，先于后续的 `Resting` 将镜像更新为新价格）。该投影保持匿名化——订单簿中没有冰山订单的隐藏储备，它也不进入增量。确定性得以保持：受影响价位集合是 `BTreeSet`，输出被显式排序，`HashMap` 的迭代不影响输出。

订阅源与引擎同址（读取订单簿）。

### 增量 L3 更新（按订单 market-by-order）— `L3Update` / `L3Feed` — [src/l3feed.rs](../../src/l3feed.rs)

L2 增量将一个价位聚合为单个数字；L3 增量工作在**单个订单**的粒度上，并携带**队列位置**。`L3Feed::apply(&events, book)` 输出 `L3Update { seq, deltas }`，其中 `L3Delta` 为 `Added { id, side, price, qty }`（订单进入其价位的**队尾**）、`Reduced { id, qty }`（新数量，位置保留）或 `Removed { id }`（订单离开订单簿）。消费者为每个价位持有一个有序的订单列表（加上 `id → 价位`）并应用增量，精确重现订单簿的 FIFO。

哪些价位受影响，订阅源**从事件中**推导——与 `L2Feed` 相同的逻辑（`Trade` 的做市方一侧；用于 `Resting` / `Modified` / `Canceled` / `Filled` 的镜像 `id → (side, price)`）。但每个受影响价位的新队列顺序**通过 `OrderBook::level_orders(side, price)` 与订单簿核对**（价位的订单 head→tail），而非从事件累计。增量是旧价位列表对新列表的差异：共同的队头子序列（保持了位置的订单）在数量变化时给出 `Reduced`，从中退出的给出 `Removed`，新列表的尾部剩余量给出 `Added`。如此，两种由事件「沉默」的情形被正确表达：**冰山订单补充**（可见 peak 耗尽 → 新 peak 进入价位队尾）和 **amend 时丧失优先级**（改变价格或增加数量）——两者都作为 remove-then-add。因此在帧内增量按 `Removed` → `Reduced` → `Added` 排序：到队尾插入时，所有退出和重排都已应用，消费者处的队列顺序与订单簿逐位一致。冰山订单的隐藏储备不进入增量（只输出可见的 peak）。

`L3Feed::new()` 从空订单簿启动，`L3Feed::from_book(book)`——以相同 `seq` 从 `l3_snapshot` 快照播种。确定性得以保持：受影响价位是按侧的价格 `BTreeSet`，`level_orders` 输出 FIFO 顺序，内部 `HashMap`（`id → 价位`、价位镜像）的迭代不影响输出。

### data 通道的端到端序列编号

所有行情数据通道都通过统一的 `command_seq(&events)`（[src/marketdata.rs](../../src/marketdata.rs) 中的单一真相源）锚定到命令的 `seq` 上：一条命令的各个事件携带共同的序列器 `seq`，每个通道用它来标记自己的帧。快照 `l2_snapshot` / `l3_snapshot` 携带 `Clob::current_seq()`（命令之后的值），增量 `L2Update` / `L3Update` 和成交记录 `TapeTrade`——携带相同的命令 `seq`。对一条命令，所有通道携带**同一个** `seq`，因此消费者：(1) 在 `seq = N` 取快照，(2) 应用 `seq > N` 的增量，(3) 按共同锚点将订单簿通道与成交带对齐。这是通道之间的契约，而非巧合：命令之后的 `current_seq()` 等于其事件的 `seq`，也就等于其所有帧的 `seq`。

### 成交带 (trade tape) — `TradeTape` / `TapeTrade` — [src/tape.rs](../../src/tape.rs)

快照和 L2 增量描述**订单簿状态**；成交带是一个独立的**执行流**通道。`TradeTape::apply(&events)` 从一条命令的事件中为每个 `Event::Trade` 提取一条匿名化成交记录 `TapeTrade { seq, price, qty, taker_side }`（按执行顺序——扫过多个价位时为 best price first），返回该命令的成交记录并将它们保存在一个有界的环形历史中（`new()`——无上限，`bounded(cap)`——最近 `cap` 条；读取——`recent()` 从旧到新、`last()`、`len()`）。每条成交记录携带命令的 `seq`——与快照和 `L2Update` 相同的锚点，因此各通道之间按 `seq` 排序。

与 L2/L3 投影不同，成交带是**无需订单簿 (book-free)** 的：`apply` 只取 `&[Event]` 而不读订单簿——成交带可由仅拥有事件流的消费者维护（这是同址 L2 订阅源目前做不到的）。这里的匿名性是关于所有者和标识符的：订单 id 不进入成交记录。但已成交的数量是公共事实，被**完整**打印：穿过冰山订单会为每个撮合的层产生一条成交记录，包括来自隐藏储备的数量。这是与订单簿投影的有意不对称——订单簿隐藏*挂着的*储备（参与者放入的内容），而成交带打印*已执行的*（市场上发生的事情）。成交带不进入状态快照（成交是流，不是状态）：在日志重放时命令被重新执行，`Trade` 事件重新产生，因此消费者从重放的流中恢复出同一条成交带。确定性得以保持——成交带只是投影一个已经确定性的事件流。

成交带是 v0.4 的最后一个通道：随着增量 L3（按订单 market-by-order）和通道的端到端序列编号，行情数据阶段宣告完成。

## 并行在哪里

在单个订单簿内部没有并行——它会破坏确定性。`Clob` 严格在单个线程中处理订单簿。

## 持久化、快照和重放 — [src/persist.rs](../../src/persist.rs)

确定性核心之上的上层结构（v0.3）。`PersistentClob` 包裹 `Clob` 并维护输入命令的**预写日志 (WAL)**：命令在事件返回给调用者**之前**就被序列化并刷到磁盘（`fsync`）——已确认的订单挺过进程崩溃。打开时，状态通过**重放**恢复：记录下来的命令流被重新提交到一个全新的 `Clob`。由于引擎是确定性的，订单簿、`order_id` 和 `seq` 被精确重现（被 gateway 拒绝的命令也会被记入日志——它们消耗 `seq`，否则重放时编号会错位）。

日志格式是自有的二进制格式，仅 `std`（没有 serde 和外部 crate）：

```
段 = [ magic "CLBW" (4) | 版本 u16 | base_seq u64 ]   头部仅一次
     然后是块:
     [ block_len u32 | rec_count varint | 记录… | crc32 u32 ]   一个块 = 一次 fsync
记录 = [ tag u8 | (对 New) 标志 side|type|tif | 按类型的 varint 字段 ]
```

数值字段为 LEB128-varint（[src/codec.rs](../../src/codec.rs)）；同一编解码器被快照复用。`Command` 的编码/解码和头部——在 [src/journal.rs](../../src/journal.rs) 中；带组提交的写入（`Journal`）和读取（`read_commands` / `read_segment`）——在 [src/wal.rs](../../src/wal.rs) 中。恢复时读取器检查 `magic`/版本和每个块的 CRC；写到一半被中断的最后一条记录（CRC 不符或块不完整）被截断——日志恢复到最后一条完整的记录。

### 检查点 (checkpoint) — [src/snapshot.rs](../../src/snapshot.rs)

为使恢复不必从头重放整个日志，`checkpoint()` 保存完整确定性状态的**快照**，然后**轮转**日志，只留下尾部：

1. `journal.commit()`——把累积的块补刷掉。
2. 原子地写入快照：写入临时文件 `<journal>.snap.tmp` → `fsync` → `rename` 为 `<journal>.snap`。这是检查点的固定点。
3. `journal.rotate(N)`——用一个 `base_seq = N`（`N` 是当前 `seq`）的全新段替换日志，同样经由临时文件 + `rename`。旧命令（`seq ≤ N`）现在只在快照中。

快照序列化所有影响后续处理的内容：序列器计数器 `seq` 和 `next_order_id`（后者不由 `seq` 推导——`Cancel`/`Modify` 消耗 `seq` 但不消耗 `order_id`）、最近成交价（止损触发所需）、所有挂着的订单按优先级顺序连同冰山订单的隐藏储备、休眠止损单以及账户的净持仓（用于持仓限额；账户按 `AccountId` 排序）。格式为 `magic "CLBS"` + 版本（`3`）、varint 字段、末尾带 CRC32 的单一帧。`RiskConfig` 不写入快照——应用在 `open_with_risk` 时给定（重放时需要相同的配置）；按账户的敞口量不存储——它通过重新灌入挂着的订单恢复。

带快照的恢复（`PersistentClob::open`）：

```
open(journal, snapshot):
    (clob, applied) = 有快照 ? (restore(快照), 快照.seq) : (Clob::new(), 0)
    (base_seq, commands) = read_segment(journal)
    对第 i 条命令:  seq = base_seq + i + 1
        if seq > applied:  clob.submit_into(命令)   # 只重放尾部
    journal = Journal::open_base(journal, applied)      # 空/新段会得到 base = applied
```

**对账按 `seq` 幂等**——这正是检查点的崩溃安全。只应用 `seq` 大于快照覆盖范围的命令，因此 `rotate` 是否来得及执行都无关紧要：如果检查点在写入快照之后、轮转之前被中断，日志仍然包含 `seq ≤ N` 的命令——它们只是被跳过（而不是被重复应用）。`seq > N` 的命令总是在日志中并被重放。损坏的快照（CRC 不符）作为 `CorruptSnapshot` 被拒绝：那时日志是唯一的真相源，不能悄悄替换状态。

确定性不变量没有被破坏：持久化层做输入输出，但不向核心引入时钟、线程或依赖 `HashMap` 的迭代（快照按价位并在价位内按 FIFO 遍历订单——确定性顺序）；分帧、提交和检查点由记录计数器和应用的显式调用控制，而非由墙上时钟。

## 模块图

| 模块 | 内容 |
| --- | --- |
| [src/types.rs](../../src/types.rs) | 原语：`OrderId`、`AccountId`、`Price`、`Qty`、`Side`、`OrderType`、`TimeInForce`、`StpMode` |
| [src/order.rs](../../src/order.rs) | 输入 API：`Command`、`NewOrder`（含 `owner` / `stp`）、`CancelOrder`、`ModifyOrder` |
| [src/gateway.rs](../../src/gateway.rs) | Gateway 阶段；基于 `RiskConfig` 的交易前风控 |
| [src/risk.rs](../../src/risk.rs) | `RiskConfig`——交易前风控参数（tick/lot、价格带、持仓限额） |
| [src/fees.rs](../../src/fees.rs) | `FeeConfig`——maker/taker 手续费率（名义金额的 ppm，带符号）和成交手续费的计算 |
| [src/sequencer.rs](../../src/sequencer.rs) | Sequencer 阶段 |
| [src/book/mod.rs](../../src/book/mod.rs) | 订单簿：价位、slab 索引、储备、账户索引 |
| [src/book/matching.rs](../../src/book/matching.rs) | 撮合算法与自成交防范（`match_against` / `detach`） |
| [src/book/accounts.rs](../../src/book/accounts.rs) | `AccountBook`——按账户的净持仓（`i128`）和敞口量 |
| [src/slab.rs](../../src/slab.rs) | 节点的 slab 内存池和价位的侵入式 FIFO 链表（`Slab` / `PriceLevel`） |
| [src/engine.rs](../../src/engine.rs) | Matching Engine 阶段 |
| [src/stops.rs](../../src/stops.rs) | 止损单簿（触发、级联激活） |
| [src/output.rs](../../src/output.rs) | 事件模型 |
| [src/marketdata.rs](../../src/marketdata.rs) | 行情数据投影：订单簿 L2 / L3 快照、增量 L2 增量（`L2Feed`）、通道共同锚点（`command_seq`） |
| [src/l3feed.rs](../../src/l3feed.rs) | 增量按订单 market-by-order：`L3Feed` / `L3Update` / `L3Delta`——带队列位置的按订单增量 |
| [src/tape.rs](../../src/tape.rs) | 成交带：`TradeTape` / `TapeTrade`——建立在事件之上的无需订单簿 (book-free) 成交记录流 |
| [src/error.rs](../../src/error.rs) | 拒绝原因 |
| [src/clob.rs](../../src/clob.rs) | 流水线连接，公共 `Clob` |
| [src/codec.rs](../../src/codec.rs) | 低层编解码器：varint/LE 原语，CRC32 |
| [src/journal.rs](../../src/journal.rs) | 日志格式：头部（magic+版本）、`Command` 序列化 |
| [src/wal.rs](../../src/wal.rs) | WAL 存储：`Journal`（写入 + `fsync` + 轮转）、`read_segment` / `read_commands`（读取） |
| [src/snapshot.rs](../../src/snapshot.rs) | 状态快照格式（`magic "CLBS"`）：`Clob` 的捕获/恢复、原子写入 |
| [src/persist.rs](../../src/persist.rs) | `PersistentClob`——带日志、快照和重放的 `Clob` 包装器 |
