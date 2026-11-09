//! The `firsthand` command line.

use clap::Parser;

/// Firsthand: verifiable handoff packets for AI coding agents.
#[derive(Parser, Debug)]
#[command(name = "firsthand", version, about)]
struct Cli {}

fn main() {
    let _ = Cli::parse();
    println!("firsthand {}", fh_core::VERSION);
}
