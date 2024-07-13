mod dtb_tool;
use clap::{Parser, Subcommand};

use dtb_tool::{PackArgs, SplitArgs};

#[derive(Parser)]
#[command(version, about, long_about=None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Split(SplitArgs),
    Pack(PackArgs),
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Split(s) => {
            let _ = dtb_tool::dtb_split(s).unwrap();
        }
        Commands::Pack(p) => dtb_tool::dtb_pack(p),
    }
}
