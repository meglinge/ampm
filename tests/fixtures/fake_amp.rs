// A disposable CLI double for lifecycle tests. Never connects to Amp or any network.
use std::{env, fs, io::{self, Write}, thread, time::Duration};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let exe = env::current_exe().unwrap();
    if args.first().map(String::as_str) == Some("--version") {
        println!("{}", fs::read_to_string(exe.with_extension("version")).unwrap_or_else(|_| "amp-fixture 1.0 (offline test double)".into()));
        return;
    }
    if args.first().map(String::as_str) == Some("update") {
        assert_eq!(args, ["update", "--porcelain"]);
        fs::write(exe.with_extension("args"), args.join("\n")).unwrap();
        fs::write(exe.with_extension("cwd"), env::current_dir().unwrap().to_string_lossy().as_bytes()).unwrap();
        let behavior = fs::read_to_string(exe.with_extension("behavior")).unwrap_or_default();
        if behavior == "fail" { eprintln!("fixture update denied"); std::process::exit(7); }
        if behavior == "unchanged" { println!("no update needed"); return; }
        if behavior == "wait" {
            for _ in 0..1000 {
                if exe.with_extension("continue").exists() { break; }
                thread::sleep(Duration::from_millis(10));
            }
        }
        let next = fs::read_to_string(exe.with_extension("next")).expect("isolated update fixture must be configured");
        fs::write(exe.with_extension("version"), &next).unwrap();
        println!("updated {next}");
        return;
    }
    if args.first().map(String::as_str) == Some("runner") {
        if args.iter().any(|s| s.contains("rejected")) { eprintln!("fixture rejected this directory"); std::process::exit(7); }
        println!("fixture command accepted: {args:?}"); return;
    }
    if !args.iter().any(|s| s == "--no-tui") { std::process::exit(2); }
    println!("fixture startup args: {args:?}");
    eprintln!("fixture stderr captured");
    io::stdout().flush().unwrap();
    let id = &args[args.iter().position(|s| s == "--runner-id").unwrap() + 1];
    let mut journal = fs::OpenOptions::new().append(true).create(true).open("fixture-launches.txt").unwrap();
    journal.write_all(format!("{id}\n").as_bytes()).unwrap();
    if id.starts_with("exit-fixture") { std::process::exit(9); }
    if id == "clean-exit-fixture" { return; }
    if id == "recover-fixture" && !std::path::Path::new("recovered.once").exists() {
        fs::write("recovered.once", "first launch fails").unwrap();
        std::process::exit(9);
    }
    loop { thread::sleep(Duration::from_secs(1)); }
}
