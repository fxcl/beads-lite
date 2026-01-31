//! beads-lite CLI entry point.

use beads_lite::{Cli, run};
use clap::Parser;
use std::io;
use std::process;

fn main() {
    let cli = Cli::parse();
    let mut stdout = io::stdout();

    if let Err(e) = run(cli, &mut stdout) {
        eprintln!("error: {}", e);
        process::exit(1);
    }
}
