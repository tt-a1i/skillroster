use clap::Parser;

/// Local skill governance for AI agents.
#[derive(Debug, Parser)]
#[command(
    name = "skillroster",
    version,
    about = "One library. The right roster for every agent.",
    long_about = "SkillRoster is a local-first governance layer for AI Agent Skills.\n\n\
                  This repository is currently a pre-alpha scaffold; operational commands are not implemented yet.",
    arg_required_else_help = true
)]
struct Cli {}

fn main() {
    Cli::parse();
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
