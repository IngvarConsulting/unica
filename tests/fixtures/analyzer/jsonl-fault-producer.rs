// Controlled protocol faults for acceptance. This is not bsl-analyzer.
use std::{env, fs, path::PathBuf};

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.iter().any(|arg| arg == "--version") {
        println!("bsl-analyzer 0.0.0-test-fixture");
        return;
    }
    assert!(args.iter().any(|arg| arg == "analyze"), "unexpected fixture invocation");
    let root = PathBuf::from(env::var("UNICA_PLUGIN_ROOT").expect("private fixture root"));
    let case = fs::read_to_string(root.join("jsonl-fault-case")).expect("known private case file");
    match case.as_str() {
        "empty" => {}
        "invalid-event" => println!("{{ private-severity-token-1064 /private/fault-source-1064"),
        "unknown-severity" => {
            println!(r#"{{"type":"start","total_files":1,"version":"0.0.0-test-fixture"}}"#);
            println!(r#"{{"type":"file","path":"CommonModules/Пример/Ext/Module.bsl","diagnostics":[{{"code":"X","message":"private-severity-token-1064 /private/fault-source-1064","severity":"private-severity-token-1064 /private/fault-source-1064","start_line":0,"start_column":0,"end_line":0,"end_column":1,"tags":[]}}]}}"#);
        }
        _ => panic!("unknown closed fixture case"),
    }
}
