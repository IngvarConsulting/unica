// Controlled resident provider: its snapshot changes only when the process restarts.
use std::{
    env, fs,
    fs::OpenOptions,
    io::{self, BufRead, Write},
};

fn quoted(value: &str) -> String {
    let mut result = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            character if character < ' ' => {
                result.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => result.push(character),
        }
    }
    result.push('"');
    result
}

fn main() {
    let arguments = env::args().collect::<Vec<_>>();
    let snapshot = fs::read_to_string(&arguments[1]).expect("fixture module");
    let lease = arguments.get(4);
    if let Some(lease) = lease {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lease)
            .expect("the previous provider must release its lease before replacement");
    }
    writeln!(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&arguments[2])
            .expect("fixture PID log"),
        "{}",
        std::process::id()
    )
    .unwrap();
    let mode = arguments.get(3).map(String::as_str).unwrap_or("analyzer");
    for line in io::stdin().lock().lines() {
        let line = line.expect("fixture request");
        let Some(id) = line.split("\"id\":").nth(1).map(str::trim_start) else {
            continue;
        };
        let id = id
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>();
        assert!(!id.is_empty(), "numeric fixture request id");
        let text = if line.contains("\"method\":\"initialize\"") {
            println!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{}}}}");
            io::stdout().flush().unwrap();
            continue;
        } else if mode == "rlm" && line.contains("rlm_start") {
            "{\"session_id\":\"snapshot-session\",\"index\":{\"index_status\":\"ready\"}}"
                .to_string()
        } else if mode == "rlm" && line.contains("rlm_end") {
            "{}".to_string()
        } else if mode == "rlm" {
            format!(
                "{{\"stdout\":{}}}",
                quoted(&format!("[{}]", quoted(&snapshot)))
            )
        } else {
            let name = snapshot
                .split_whitespace()
                .nth(1)
                .expect("fixture procedure")
                .split('(')
                .next()
                .unwrap();
            let snippet = snapshot
                .lines()
                .map(|line| format!("│ {line}\n"))
                .collect::<String>();
            format!("#1 [L] CommonModules/SearchProbe/Ext/Module.bsl:1-2 :: {name} (procedure)\n{snippet}")
        };
        println!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":{}}}]}}}}",
            quoted(&text)
        );
        io::stdout().flush().unwrap();
    }
    if let Some(lease) = lease {
        // The supplied provider releases its cache lease after MCP EOF shutdown.
        // Killing this fixture deliberately leaves the ownership witness behind.
        if let Some(cleanup_gate) = arguments.get(5) {
            fs::write(format!("{cleanup_gate}.closing"), b"EOF received").unwrap();
            while std::path::Path::new(cleanup_gate).exists() {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        fs::remove_file(lease).expect("release provider lease after EOF");
    }
}
