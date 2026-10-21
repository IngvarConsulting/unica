pub fn run_from_args(args: &[String]) -> Result<(), String> {
    crate::infrastructure::runtime_composition::run_worker_from_args(args)
}
