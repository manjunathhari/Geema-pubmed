//! `stage0-runner`: runs the job in `STAGE0_JOB`. `stage0-runner print <label>`
//! prints one saved result for reading back through the logs.

use stage0_runner::ollama::Limits;
use stage0_runner::print::{PrintError, print_result};
use stage0_runner::run::{load, run_all};
use std::path::Path;
use std::process::ExitCode;

fn normal() -> ExitCode {
    let (cfg, jobs) = match load(|k| std::env::var(k).ok()) {
        Ok(v) => v,
        Err(e) => {
            println!("stage0 error={}", e.word());
            return ExitCode::from(2);
        }
    };
    let s = run_all(&cfg, &jobs, &Limits::REAL, &mut std::io::stdout());
    if s.saves_failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn print(label: &str) -> ExitCode {
    let Some(out) = std::env::var("STAGE0_OUT").ok().filter(|v| !v.is_empty()) else {
        println!("stage0 error=MISSING_ENV");
        return ExitCode::from(2);
    };
    match print_result(Path::new(&out), label, &mut std::io::stdout()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let word = match e {
                PrintError::BadLabel => "BAD_LABEL",
                PrintError::NoResult => "NO_RESULT",
                PrintError::Failed => "PRINT_FAILED",
            };
            println!("stage0 error={word}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    // A panic's message could quote input, so it never reaches the log.
    std::panic::set_hook(Box::new(|_| println!("stage0 error=PANIC")));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => normal(),
        ["print", label] => print(label),
        _ => {
            println!("stage0 error=USAGE");
            ExitCode::from(2)
        }
    }
}
