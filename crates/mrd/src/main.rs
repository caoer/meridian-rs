//! Process wiring for the `mrd` bin: collect argv and hand off to `mrd::run`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    mrd::run(&args)
}
