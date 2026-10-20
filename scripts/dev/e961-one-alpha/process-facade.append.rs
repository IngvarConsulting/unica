
// Temporary E961 diagnostic only: borrow the already owned process object.
#[cfg(all(test, target_os = "windows"))]
impl ManagedChild {
    pub(crate) fn e961_with_owned_process_handle<R>(
        &self,
        inspect: impl FnOnce(std::os::windows::io::BorrowedHandle<'_>) -> R,
    ) -> R {
        use std::os::windows::io::AsHandle;
        inspect(self.child.as_handle())
    }
}
