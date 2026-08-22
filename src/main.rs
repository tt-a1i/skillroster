use clap::Parser;
use skillroster::{app, cli::Cli};

fn main() {
    let wants_json = std::env::args_os().any(|argument| argument == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if wants_json => {
            println!("{}", app::error_json("cli", &error));
            std::process::exit(2);
        }
        Err(error) => error.exit(),
    };
    let json = cli.json;
    let command = cli.command_name();

    match app::run(cli) {
        Ok(output) => println!("{}", if json { output.json } else { output.human }),
        Err(error) => {
            if json {
                println!("{}", app::error_json(command, error.as_ref()));
            } else {
                eprintln!("{}", skillroster::present::error_human(&error));
            }
            std::process::exit(1);
        }
    }
}
