//! Portable transport for a controlled test producer, never a published runner.
use std::{env, fs, process::{self, Command}};

fn main() {
    let executable = env::current_exe().expect("fixture executable");
    let directory = executable.parent().expect("fixture directory");
    let python = fs::read_to_string(directory.join("python-path")).expect("fixture interpreter");
    let status = Command::new(python)
        .arg(directory.join("producer.py"))
        .args(env::args_os().skip(1))
        .status()
        .expect("controlled fixture producer");
    process::exit(status.code().unwrap_or(1));
}
