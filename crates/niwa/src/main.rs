use clap::Parser;
use niwa_garden::cli::{execute, Cli};
use niwa_garden::ingest::try_run_fast_cli_from_env;
use niwa_garden::prompt::try_run_fast_prompt_from_env;
use std::process::ExitCode;

fn main() -> ExitCode {
    if let Some(code) = try_run_fast_cli_from_env() {
        return code;
    }
    if let Some(code) = try_run_fast_prompt_from_env() {
        return code;
    }
    let cli = Cli::parse();
    match execute(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
