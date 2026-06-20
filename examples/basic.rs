use clob::{Clob, Command, NewOrder, Side};

fn print_events(label: &str, events: &[clob::Event]) {
    println!("{label}");
    for event in events {
        println!("  {event:?}");
    }
}

fn main() {
    let mut clob = Clob::new();

    print_events(
        "post ask 101 x10",
        &clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 10))),
    );
    print_events(
        "post ask 102 x5",
        &clob.submit(Command::New(NewOrder::limit(Side::Sell, 102, 5))),
    );
    print_events(
        "post bid 100 x7",
        &clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 7))),
    );

    print_events(
        "aggressive bid 101 x12 (crosses)",
        &clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 12))),
    );

    let book = clob.book();
    println!();
    println!("best bid = {:?}", book.best_bid());
    println!("best ask = {:?}", book.best_ask());
    println!("spread   = {:?}", book.spread());
    println!("asks     = {:?}", book.depth(Side::Sell, 5));
    println!("bids     = {:?}", book.depth(Side::Buy, 5));
}
