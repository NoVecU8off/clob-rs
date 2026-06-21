# 路线图

各阶段为大致安排，顺序可能调整。标记：`[ ]` — 未开始，
`[~]` — 进行中，`[x]` — 已完成。当前状态见 [STATUS.md](STATUS.md)。

## crate 范围

`clob` — 纯粹的可嵌入撮合引擎：仅依赖 `std`，确定性的
单线程核心，输出事件。核心已经是一个**完成且经过测试的
块**（v0.2–v0.4 阶段已关闭）。crate 自身的后续
工作——只做基于 `std` 的领域完整性。

## v0.2 — 性能与订单类型

- [x] **价位上的侵入式链表** → 价位内撤单为 `O(1)`
      （节点位于带空闲槽列表的 slab 内存池中；移除了原先的 `O(n)` 扫描）。
- [x] **`Modify` / amend 命令** — 修改价格/数量；改价或
      增大数量会丧失时间优先级，减小则保留。
- [x] **Post-only** — 若订单会立即穿越价差则拒绝（maker-only）。
- [x] **Stop / stop-limit** — 延迟订单，按触发价激活
      （以最后成交价为准；触发为级联式，事件为 `Triggered`）。
- [x] **Iceberg** — 数量的可见（peak）与隐藏（reserve）部分；当
      可见部分耗尽时，peak 从储备补充至价位尾部（丧失优先级）。
- [x] 撮合中的可重用分配池（带空闲槽列表的 slab；
      移除了用于已成交 id 的 `Vec`）。

## v0.3 — 可靠性与重放

- [x] **事件日志（event sourcing）** — 将输入命令流写入 append-only
      WAL（`Journal`）：自有的二进制格式，采用 varint、逐帧 CRC32、带 `fsync` 的组提交。
      封装器 `PersistentClob` 将命令以 write-ahead 方式记入日志——在返回事件前即持久（durable）。
- [x] **重放** — 通过将日志重新送入全新的 `Clob` 来恢复状态
      （`PersistentClob::open`）；得益于确定性，订单簿、`order_id` 和 `seq` 被精确
      重现。中断写入产生的损坏尾部按 CRC 截断。热备（副本）——
      用同一套读取命令流的机制。
- [x] 订单簿状态的**快照（snapshots）**，用于加速恢复（`checkpoint()`
      写入完整状态的快照并轮转日志；`open` 加载快照并仅重放
      日志尾部；协调按 `seq` 幂等，检查点崩溃安全）。*(v0.3 第 2 阶段)*
- [x] 格式版本化：日志 — 段头中的 `magic "CLBW"` + 版本；
      快照 — `magic "CLBS"` + 版本，共用带 CRC32 的帧。

## v0.4 — 行情数据 (market data)

- [x] **L2 快照**（按价位聚合的深度）与 **L3**（按订单）——
      `Clob::l2_snapshot` / `Clob::l3_snapshot`，二者均以 `seq` 标记；只读投影位于
      `src/marketdata.rs`，冰山订单的隐藏储备不进入快照。
- [x] **L2 增量更新**（market-by-price）：仅含已变更
      价位的增量（`L2Update` / `L2Feed`），叠加于共享 `seq` 的快照之上；核心不受影响。
- [x] **L3 增量更新**（market-by-order，含队列位置）：每订单的
      增量 `L3Delta`（`Added` / `Reduced` / `Removed`），经 `L3Feed` 装入 `L3Update`
      （`src/l3feed.rs`）。受影响的价位由事件推导（与 L2 相同），而价位
      内的队列顺序与订单簿核对（`OrderBook::level_orders`）；冰山补充及 amend 时的丧失
      优先级表现为 remove-then-add（移至价位尾部）。
- [x] **成交带 (trade tape)** 作为独立通道 —— `TradeTape` 从事件流中提取
      公开的匿名化成交记录（`TapeTrade { seq, price, qty, taker_side }`，每个
      `Trade` 事件对应一条）并维护一段有限的环形历史。该通道**无需订单簿 (book-free)**：`apply` 仅取
      事件，不访问订单簿。成交记录携带命令的 `seq`——与订单簿通道共享的锚点。
- [x] **各数据通道之间序列号的一致性** —— 所有通道（L2/L3
      快照、`L2Update` / `L3Update` 增量、`TapeTrade` 成交记录）通过统一的 `command_seq`
      锚定于命令的 `seq`：对于同一条命令，所有通道携带同一个 `seq`，等于
      其后的 `Clob::current_seq()`。消费者应用 `seq` 大于
      快照的增量，并按共享的 `seq` 将各通道相互对齐。

## v0.5 — 引擎的领域完整性（仅 `std`）

引擎自身的领域逻辑——不带外部依赖，核心保持纯粹且
确定性。

- [x] **订单的账户/所有者**与 **self-trade prevention**（自成交防范）：
      订单模型中的 `owner: AccountId` + `stp: StpMode`；STP 在撮合时按**主动方**
      策略（cancel-taker / cancel-maker / cancel-both）触发，针对每个对手挂单做市方 (maker)。
      `owner` 经历快照/重放并不泄露到行情数据中。`decrement` 策略（按 overlap 削减
      双方）与 `Modify` 上的 STP——已推迟。
- [x] Gateway 中的 **Pre-trade risk**：`RiskConfig`（tick/lot、价格带、持仓限额），
      经 `Clob::with_risk` / `PersistentClob::open_with_risk` 接入，默认关闭。
      tick/lot——价格/数量的整数倍；价格带——以 mid `(best_bid+best_ask)/2` 为中心的 `±band_ticks`
      （订单簿冷启动时跳过；止损单/market 不受限）；持仓限额——带符号净额的最坏情况
      （`net + 该方向敞口量 + qty ≤ limit`），仅对 `owner != 0` 生效。净持仓/敞口
      量由订单簿记账；净持仓经历快照（`CLBS` → v3）与重放。STP-`Modify` 与手续费——另行处理。
- [x] **手续费** maker/taker 及其在成交事件中的计算：`FeeConfig`（maker/taker 费率
      以 **ppm** 计——名义金额 `price·qty` 的百万分之），**带符号**（`i64`）——负
      maker 费率即返佣 (rebate)。经 `Clob::with_fees` / `with_risk_and_fees` /
      `PersistentClob::open_with_fees` / `open_with_risk_and_fees` 接入，默认关闭。手续费在
      每笔成交上计算并输出到 `Event::Trade`（`taker_fee` / `maker_fee`）；取整方式——
      向零截断。核心**不**累积手续费状态（events out, not state in），快照不
      变更，`FeeConfig` 不持久化（重放时——同一配置）。v0.5 阶段已关闭。

## 核心发布

核心是一个完成的模块，是首个公开发布的合理节点（`1.0` 候选）。

- [ ] 在 `Cargo.toml` 中提升版本，打上 git 标签，对齐
      [CHANGELOG.md](CHANGELOG.md) 中的链接。
- [ ] 对整个公开 API 的 **Rustdoc** + 发布到 docs.rs。
- [ ] 发布到 crates.io（清单已就绪：description / keywords /
      categories / license）。
- [ ] **许可证文件** `LICENSE-MIT` 与 `LICENSE-APACHE`。

## 质量与工具

这些条目会引入外部 crate，因此**不计入**核心依赖——它们作为
仅 `std` 的示例或独立的 dev 测试套件存在（`CLAUDE.md` 中的 zero-dependency 规则）。

- [ ] 延迟基准测试（p50/p99/p99.9）——作为基于 `std::time` 的 `examples/`
      （不将 criterion 添加为依赖）。
- [ ] 命令流水线的 **Property-based 测试**与**模糊测试**——作为 crate
      之外的独立 dev 测试套件（proptest / fuzz 会引入依赖）。
