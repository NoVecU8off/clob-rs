use clob::{Command, NewOrder, PersistentClob, Side};

fn print_book(label: &str, clob: &PersistentClob) {
    let book = clob.book();
    println!("{label}");
    println!("  seq           = {}", clob.current_seq());
    println!(
        "  best bid/ask  = {:?} / {:?}",
        book.best_bid(),
        book.best_ask()
    );
    println!("  bids          = {:?}", book.depth(Side::Buy, 5));
    println!("  asks          = {:?}", book.depth(Side::Sell, 5));
    println!("  pending stops = {}", clob.pending_stops());
}

fn main() {
    let mut path = std::env::temp_dir();
    path.push(format!("clob_replay_demo_{}.wal", std::process::id()));
    let _ = std::fs::remove_file(&path);

    {
        let mut clob = PersistentClob::open(&path).expect("open journal");
        clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 10)))
            .expect("submit");
        clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 7)))
            .expect("submit");
        clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 4)))
            .expect("submit");
        clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 103, 30, 5)))
            .expect("submit");
        clob.submit(Command::New(NewOrder::stop(Side::Buy, 110, 5)))
            .expect("submit");
        print_book("session 1 (live)", &clob);
    }

    println!("\n--- process restarts; book is only in the journal on disk ---\n");

    let mut clob = PersistentClob::open(&path).expect("recover journal");
    print_book("session 2 (recovered by replaying the journal)", &clob);

    println!();
    clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 2)))
        .expect("submit");
    print_book("session 2 continues writing seamlessly", &clob);

    let _ = std::fs::remove_file(&path);
}
