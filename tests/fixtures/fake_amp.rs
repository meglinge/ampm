// A disposable CLI double for lifecycle tests. Never connects to Amp or any network.
use std::{env, io::{self, Write}, thread, time::Duration};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") { println!("amp-fixture 1.0 (offline test double)"); return; }
    if args.first().map(String::as_str) == Some("runner") {
        if args.iter().any(|s| s.contains("rejected")) { eprintln!("fixture rejected this directory"); std::process::exit(7); }
        println!("fixture command accepted: {args:?}"); return;
    }
    if !args.iter().any(|s| s == "--no-tui") { std::process::exit(2); }
    println!("fixture startup args: {args:?}");
    eprintln!("fixture stderr captured");
    io::stdout().flush().unwrap();
    if args.iter().any(|s| s == "exit-fixture") { std::process::exit(9); }
    loop { thread::sleep(Duration::from_secs(1)); }
}
