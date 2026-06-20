use std::time::Instant;

use clob::{Clob, Command, NewOrder, Side};

fn main() {
    let total: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);

    let mut clob = Clob::new();
    let mut scratch = Vec::with_capacity(8);

    let start = Instant::now();
    for i in 0..total {
        let side = if i % 2 == 0 { Side::Buy } else { Side::Sell };
        let price = 90 + (i % 21);
        scratch.clear();
        clob.submit_into(Command::New(NewOrder::limit(side, price, 1)), &mut scratch);
    }
    let elapsed = start.elapsed();

    let per_sec = total as f64 / elapsed.as_secs_f64();
    println!("submitted {total} orders in {elapsed:?}");
    println!("throughput: {per_sec:.0} orders/s");
    println!("resting orders left on book: {}", clob.book().len());
}
