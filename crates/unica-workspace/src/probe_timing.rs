//! Throwaway timing probe for #1879; never merged.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub const NAMES: [&str; 12] = [
    "source_admission",
    "open_absolute_dir_nofollow",
    "state_handle_sync_all",
    "publish_record_batch",
    "retire_batch_backings",
    "create_state_file_child",
    "remove_state_regular_child",
    "rename_no_replace",
    "verify_named_authority",
    "normalize_path_identity",
    "sync_receipt_row_directory",
    "host_bind",
];
static NANOS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
static CALLS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];

pub struct Span(usize, Instant);
pub fn span(slot: usize) -> Span {
    Span(slot, Instant::now())
}
impl Drop for Span {
    fn drop(&mut self) {
        NANOS[self.0].fetch_add(self.1.elapsed().as_nanos() as u64, Ordering::Relaxed);
        CALLS[self.0].fetch_add(1, Ordering::Relaxed);
    }
}
pub fn report() -> String {
    let mut out = String::new();
    for (i, name) in NAMES.iter().enumerate() {
        let n = NANOS[i].load(Ordering::Relaxed);
        let c = CALLS[i].load(Ordering::Relaxed);
        out.push_str(&format!(
            "PROBE {name}: calls={c} total_ms={} avg_us={}\n",
            n / 1_000_000,
            if c == 0 { 0 } else { n / c / 1_000 }
        ));
    }
    out
}
