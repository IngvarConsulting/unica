pub fn run_from_args(args: &[String]) -> Result<(), String> {
    crate::infrastructure::analysis_composition::run_workspace_service(args)
}
