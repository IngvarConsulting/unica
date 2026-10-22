//! Fixed producer for the full-pull guard's real-process completion test.
//! It writes only its own stage and fixture journal; it is not a platform emulator.
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::time::Duration;

fn json_string(value: &str) -> String {
    let mut text = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => text.push_str("\\\\"),
            '"' => text.push_str("\\\""),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            control if control <= '\u{1f}' => {
                text.push_str(&format!("\\u{:04x}", u32::from(control)));
            }
            other => text.push(other),
        }
    }
    text.push('"');
    text
}

fn frame(value: &str) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{value}")?;
    stdout.flush()
}

fn record(path: &Path, nonce: &str, event: &str, fields: &str) -> io::Result<()> {
    let mut journal = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        journal,
        "{{\"event\":{},\"pid\":{},\"nonce\":{}{fields}}}",
        json_string(event),
        std::process::id(),
        json_string(nonce)
    )
}

fn ack(nonce: &str, phase: &str, decision: &str) -> io::Result<String> {
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "caller closed stdin before ACK",
        ));
    }
    let compact: String = line
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    for field in [
        "\"version\":1".to_string(),
        format!("\"nonce\":{}", json_string(nonce)),
        format!("\"phase\":{}", json_string(phase)),
        format!("\"decision\":{}", json_string(decision)),
    ] {
        if !compact.contains(&field) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected publication ACK",
            ));
        }
    }
    Ok(line.trim().to_string())
}

fn main() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let config = fs::read_to_string(executable.with_extension("fixture"))?;
    let fields: Vec<_> = config.lines().collect();
    if fields.len() != 5 || !matches!(fields[0], "complete" | "missing-terminal" | "withheld-eof") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid fixed producer fixture",
        ));
    }
    let mode = fields[0];
    let args: Vec<_> = std::env::args().skip(1).collect();
    let stage = args
        .windows(2)
        .find(|pair| pair[0] == "--publication-stdio")
        .map(|pair| Path::new(&pair[1]))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no caller-owned stage"))?;
    let preview = args.iter().any(|argument| argument == "--dry-run");
    let journal = executable.with_extension("events.jsonl");
    // Each invocation has its own nonce. Cryptographic randomness is not under test.
    let nonce = format!("00000000-0000-4000-8000-{:012x}", std::process::id());
    let identity = format!("\"version\":1,\"nonce\":{}", json_string(&nonce));
    writeln!(io::stderr().lock(), "publication producer stderr")?;
    io::stderr().flush()?;
    frame(&format!(
        "{{{identity},\"phase\":\"selected\",\"provider\":{{\"selected\":\"designer\"}},\"executor\":{{\"kind\":\"local\",\"executable\":{},\"endpoint\":null}}}}",
        fields[1]
    ))?;
    let selected_ack = ack(&nonce, "selected", "continue")?;
    record(
        &journal,
        &nonce,
        "selected_ack",
        &format!(",\"reply\":{selected_ack}"),
    )?;
    if preview {
        frame(&format!(
            "{{{identity},\"phase\":\"terminal\",\"publication\":\"not_published\",\"memory\":\"unchanged\",\"result\":{}}}",
            fields[3]
        ))?;
        return Ok(());
    }
    fs::create_dir_all(stage.join("Catalogs"))?;
    fs::write(stage.join("Configuration.xml"),
        b"<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Configuration><Properties/></Configuration></MetaDataObject>")?;
    fs::write(stage.join("Catalogs/Items.xml"),
        b"<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Catalog><Properties/></Catalog></MetaDataObject>")?;
    let encoded_stage =
        json_string(stage.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 fixture stage")
        })?);
    record(
        &journal,
        &nonce,
        "ready",
        &format!(",\"stage\":{encoded_stage}"),
    )?;
    frame(&format!(
        "{{{identity},\"phase\":\"ready\",\"command\":\"pull\",\"requested_mode\":\"FULL\",\"mode\":\"FULL\",\"mode_reason\":null,\"extension\":null,\"provider_dispatched\":true,\"source_set\":\"main\",\"target_path\":{},\"stage_path\":{encoded_stage},\"snapshot\":\"prepared\",\"snapshot_warning\":null}}",
        fields[2]
    ))?;
    let published_ack = ack(&nonce, "ready", "published")?;
    record(
        &journal,
        &nonce,
        "published_ack",
        &format!(",\"reply\":{published_ack}"),
    )?;
    if mode == "missing-terminal" {
        record(&journal, &nonce, "exit_without_terminal", "")?;
        return Ok(());
    }
    frame(&format!(
        "{{{identity},\"phase\":\"terminal\",\"publication\":\"published\",\"memory\":\"saved\",\"result\":{}}}",
        fields[4]
    ))?;
    record(&journal, &nonce, "terminal_sent", "")?;
    if mode == "withheld-eof" {
        record(&journal, &nonce, "eof_withheld", "")?;
        loop {
            std::thread::park_timeout(Duration::from_secs(30));
        }
    }
    Ok(())
}
