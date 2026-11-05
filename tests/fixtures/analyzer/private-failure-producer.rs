use std::{
    env, fs,
    io::{self, BufRead, Write},
};

fn main() {
    let mut args = env::args().skip(1);
    let pid = args.next().unwrap();
    let route = args.next().unwrap();
    let mode = args.next().unwrap();
    fs::write(pid, std::process::id().to_string()).unwrap();
    for (index, line) in io::stdin().lock().lines().enumerate() {
        let line = line.unwrap();
        match index {
            0 => {
                assert!(line.contains("\"method\":\"initialize\""));
                fs::write(&route, "initialize\n").unwrap();
                if mode == "malformed" {
                    eprintln!("PRIVATE_SECRET_HELPER_STDERR");
                }
                println!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}");
            }
            1 => {
                assert!(line.contains("\"method\":\"notifications/initialized\""));
                fs::write(&route, "initialize\ninitialized\n").unwrap();
            }
            2 => {
                assert!(line.contains("\"method\":\"tools/call\""));
                fs::write(&route, "initialize\ninitialized\nactive\n").unwrap();
                if mode == "stall" {
                    // The caller gives up on this call; its next one is answered.
                } else if mode == "malformed" {
                    println!("PRIVATE_SECRET_PROTOCOL_PAYLOAD{{");
                    io::stdout().flush().unwrap();
                    // The next stdin read keeps the process alive until the owner
                    // terminates it. Worker death also closes this pipe, so the
                    // controlled helper cannot outlive a failed worker indefinitely.
                } else {
                    println!("{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"valid\"}}]}}}}");
                }
            }
            _ if line.contains("\"method\":\"notifications/cancelled\"") => {
                assert!(line.contains("\"requestId\":2"), "{line}");
                let mut seen = fs::read_to_string(&route).unwrap();
                seen.push_str("cancelled\n");
                fs::write(&route, seen).unwrap();
            }
            _ => {
                println!("{{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"valid\"}}]}}}}");
            }
        }
        io::stdout().flush().unwrap();
    }
}
