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

    println!();
    print_events(
        "buy-stop trigger 102 x4 (parks; becomes market on trigger)",
        &clob.submit(Command::New(NewOrder::stop(Side::Buy, 102, 4))),
    );
    println!("pending stops = {}", clob.pending_stops());
    print_events(
        "buy 102 x1 -> prints @102 and triggers the stop",
        &clob.submit(Command::New(NewOrder::limit(Side::Buy, 102, 1))),
    );
    println!("pending stops = {}", clob.pending_stops());
    println!("asks          = {:?}", clob.book().depth(Side::Sell, 5));
}
