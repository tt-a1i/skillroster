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
    // Apply and Undo start progress inside `app::run`, after human confirmation.
    let progress = (!matches!(command, "apply" | "undo"))
        .then(|| skillroster::present::ProgressGuard::start(command, json));

    match app::run(cli) {
        Ok(output) => {
            if let Some(progress) = progress {
                progress.finish();
            }
            println!("{}", if json { output.json } else { output.human });
        }
        Err(error) => {
            if let Some(progress) = progress {
                progress.finish();
            }
            if json {
                println!("{}", app::error_json(command, error.as_ref()));
            } else {
                eprintln!("{}", skillroster::present::error_human(&error));
            }
            std::process::exit(1);
        }
    }
}
