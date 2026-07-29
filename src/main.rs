use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "cortana", version, about = "Agent-native second brain")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print runtime and configuration health.
    Doctor,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Doctor) => println!("cortana: bootstrap healthy"),
        None => println!("cortana {}", env!("CARGO_PKG_VERSION")),
    }
}
