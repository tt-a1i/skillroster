use std::io::{self, Write};

use clap::Parser;
use skillroster::{app, cli::Cli};

fn write_stdout_or_exit(output: &str) {
    let result = {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        stdout
            .write_all(output.as_bytes())
            .and_then(|()| stdout.write_all(b"\n"))
    };
    if let Err(error) = result {
        if error.kind() == io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        let _ = writeln!(
            io::stderr().lock(),
            "failed writing command output: {error}"
        );
        std::process::exit(1);
    }
}

fn main() {
    let wants_json = std::env::args_os().any(|argument| argument == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) if wants_json => {
            write_stdout_or_exit(&app::error_json("cli", &error));
            std::process::exit(2);
        }
        Err(error) => error.exit(),
    };
    let json = cli.json;
    let command = cli.command_name();
    let action_context = app::ActionContext::from_cli(&cli);
    // Apply and Undo start progress inside `app::run`, after human confirmation.
    let progress = (!matches!(command, "apply" | "undo"))
        .then(|| skillroster::present::ProgressGuard::start(command, json));

    match app::run(cli) {
        Ok(output) => {
            if let Some(progress) = progress {
                progress.finish();
            }
            write_stdout_or_exit(if json { &output.json } else { &output.human });
        }
        Err(error) => {
            if let Some(progress) = progress {
                progress.finish();
            }
            if json {
                write_stdout_or_exit(&app::error_json_with_context(
                    command,
                    error.as_ref(),
                    &action_context,
                ));
            } else if let Some(blocked) =
                error.downcast_ref::<skillroster::roster_plan::RosterPlanBlocked>()
            {
                eprintln!(
                    "{}",
                    skillroster::present::blocked_roster_plan(&blocked.details)
                );
            } else {
                eprintln!("{}", skillroster::present::error_human(&error));
            }
            std::process::exit(1);
        }
    }
}
