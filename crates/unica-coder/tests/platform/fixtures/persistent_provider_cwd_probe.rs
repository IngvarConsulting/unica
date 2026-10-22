use std::{
    env, fs,
    io::{self, BufRead, Write},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
fn native_hex(value: &std::ffi::OsStr) -> String {
    use std::os::unix::ffi::OsStrExt;
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
#[cfg(windows)]
fn native_hex(value: &std::ffi::OsStr) -> String {
    use std::os::windows::ffi::OsStrExt;
    value
        .encode_wide()
        .map(|unit| format!("{unit:04x}"))
        .collect()
}

fn observation() -> String {
    let cwd = env::current_dir().unwrap();
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(".").unwrap();
        format!("{};{}", metadata.dev(), metadata.ino())
    };
    #[cfg(windows)]
    let identity = "not-unix;not-unix".to_string();
    let generation = env::var_os("RLM_INDEX_DIR")
        .map(|value| native_hex(&value))
        .unwrap_or_default();
    let args = env::args_os()
        .skip(1)
        .map(|value| native_hex(&value))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{};{};{};{};{}",
        std::process::id(),
        native_hex(cwd.as_os_str()),
        generation,
        identity,
        args
    )
}

fn helper_mode() {
    let ready = PathBuf::from(env::var_os("UNICA_CWD_PROBE_READY").unwrap());
    let ping = PathBuf::from(env::var_os("UNICA_CWD_PROBE_PING").unwrap());
    let pong = PathBuf::from(env::var_os("UNICA_CWD_PROBE_PONG").unwrap());
    let stop = PathBuf::from(env::var_os("UNICA_CWD_PROBE_STOP").unwrap());
    fs::write(ready, format!("{}\n", observation())).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut answered = false;
    while Instant::now() < deadline {
        if stop.exists() {
            return;
        }
        if !answered && fs::read(&ping).ok().as_deref() == Some(b"challenge-after-rename") {
            fs::write(&pong, format!("{}\n", observation())).unwrap();
            answered = true;
        }
        thread::sleep(Duration::from_millis(2));
    }
    std::process::exit(91);
}

fn request_id(line: &str) -> Option<u64> {
    let start = line.find("\"id\":")? + 5;
    let digits = line[start..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

fn main() {
    if env::var_os("UNICA_CWD_PROBE_HELPER").is_some() {
        helper_mode();
        return;
    }
    let (sender, receiver) = mpsc::sync_channel(8);
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if sender.send(line).is_err() {
                return;
            }
        }
    });
    loop {
        let line = match receiver.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(line)) => line,
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => std::process::exit(92),
        };
        let Some(id) = request_id(&line) else {
            continue;
        };
        if line.contains("\"method\":\"initialize\"") {
            println!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{}}}}");
        } else {
            println!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}", observation());
        }
        io::stdout().flush().unwrap();
    }
}
