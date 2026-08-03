mod add;
mod check;
mod checksum;
mod config;
mod config_edit;
mod init;
mod rule;
#[cfg(test)]
mod testdata;

use checksum::ChecksumAlgorithm;
use clap::{Parser, Subcommand};
use rule::Severity;

#[derive(Parser, Debug)]
#[command(
    name = "ruleman",
    version,
    about = "Repository static analysis by declarative rules"
)]
struct Cli {
    /// Path to the config file. When omitted, ruleman.json / ruleman.jsonc / .ruleman.json
    /// is discovered starting from the current directory and walking up.
    #[arg(long, global = true)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Scaffold a starter ruleman.json in the current directory.
    Init {
        #[arg(long)]
        force: bool,
    },
    /// Add existing files/directories to the config as rules.
    ///
    /// Each path must exist; whether it becomes a `file` or a `directory`
    /// rule is decided by what's on disk. Paths are stored relative to the
    /// config file, and comments/formatting in the config are preserved.
    ///
    /// With --checksum the file's current hash is recorded as a `checksum`
    /// rule instead; re-running it after an intentional edit refreshes the
    /// recorded hash.
    Add {
        /// Paths to add, relative to the current directory.
        #[arg(required = true)]
        paths: Vec<String>,

        /// Severity of the rule the paths are added to.
        #[arg(long, value_enum, default_value_t = Severity::Error)]
        severity: Severity,

        /// Record each file's current hash as a `checksum` rule instead of an
        /// existence rule.
        #[arg(long)]
        checksum: bool,

        /// Hash algorithm to record with. Defaults to sha256.
        #[arg(long, value_enum, requires = "checksum")]
        algorithm: Option<ChecksumAlgorithm>,
    },
}

fn main() {
    let cli = Cli::parse();
    let code = match cli.command {
        Some(Command::Init { force }) => init::run(force),
        Some(Command::Add {
            paths,
            severity,
            checksum,
            algorithm,
        }) => add::run(
            cli.config.as_deref(),
            &paths,
            severity,
            checksum.then(|| algorithm.unwrap_or_default()),
        ),
        None => check::run(cli.config.as_deref()),
    };
    std::process::exit(code);
}
