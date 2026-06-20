# Журнал обновлений

Формат основан на [Keep a Changelog](https://keepachangelog.com/ru/1.1.0/),
проект следует [семантическому версионированию](https://semver.org/lang/ru/).

## [Unreleased]

Запланированное см. в [ROADMAP.md](ROADMAP.md).

## [0.1.0] — 2026-06-20

Первый релиз. Детерминированное ядро CLOB с конвейером
`Gateway → Sequencer → Matching Engine → Output`.

### Добавлено

- **Конвейер `Clob`** — единая точка входа `submit()` / `submit_into()`, связывающая все стадии.
- **Gateway** — валидация заявок: отказ при нулевом количестве (`ZeroQuantity`) и нулевой цене лимитной заявки (`InvalidPrice`).
- **Sequencer** — монотонные номера последовательности и идентификаторы заявок для детерминированного порядка.
- **Matching Engine** — сведение по price-time priority (FIFO внутри ценового уровня).
- **Order book** — `BTreeMap` ценовых уровней по каждой стороне, `VecDeque` для FIFO внутри уровня, `HashMap` индекс заявок для отмены.
- **Типы заявок** — `Limit`, `Market`.
- **Time-in-force** — `Gtc`, `Ioc`, `Fok`.
- **События** — `Accepted`, `Trade`, `Resting`, `Filled`, `Canceled`, `Rejected`.
- **Доступ к книге** — `best_bid`, `best_ask`, `spread`, `depth`, `len`, `contains`, `available_qty`.
- **Примеры** — `examples/basic.rs` (демонстрация событий и книги), `examples/throughput.rs` (нагрузочный прогон).
- **Тесты** — 12 интеграционных тестов на матчинг, приоритет, TIF, отмену и инварианты книги.
- **Документация** — папка `docs/` (статус, архитектура, журнал обновлений, roadmap).

### Особенности реализации

- Целочисленные цены и количества (`u64`), без чисел с плавающей точкой.
- Ядро без внешних зависимостей (только `std`).
- Редакция Rust 2024; `release`-профиль с LTO и `panic = "abort"`.

[Unreleased]: https://example.com/clob/compare/v0.1.0...HEAD
[0.1.0]: https://example.com/clob/releases/tag/v0.1.0
