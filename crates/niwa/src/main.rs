use clap::Parser;
use tg_core::product;

#[derive(Parser, Debug)]
#[command(
    name = product::CLI_BIN,
    bin_name = product::CLI_BIN,
    version = product::VERSION,
    about = "A cozy idle garden for your terminal and menu bar"
)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
