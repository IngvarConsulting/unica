pub fn run_from_args(args: &[String]) -> Result<(), String> {
    crate::infrastructure::runtime_jobs::run_worker_from_args(args)
}
