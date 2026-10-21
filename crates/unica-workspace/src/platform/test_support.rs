#[cfg(unix)]
use super::filesystem::execution_store;

pub struct StageWriteLimitGuard {
    #[cfg(unix)]
    _guard: execution_store::StageWriteLimitGuard,
}

pub fn install_stage_write_limit(bytes: usize) -> StageWriteLimitGuard {
    #[cfg(unix)]
    {
        StageWriteLimitGuard {
            _guard: execution_store::install_stage_write_limit(bytes),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = bytes;
        panic!("the existing stage-write-limit fixture requires a Unix host");
    }
}

#[cfg(unix)]
pub use super::testing::ForkedInheritedDescriptorForTest;
