//! The driving adapter: parses the command line and dispatches to the commands.

mod commands;

use clap::Parser;

use crate::adapters::cli_support::args::{Cli, Cmd};
use crate::adapters::cli_support::session::Session;
use crate::domain::error::{Error, Result};

/// The process exit code of an error.
fn exit_code(e: &Error) -> i32 {
    match e {
        Error::Internal(_) | Error::Git(_) | Error::Io(_) => 1,
        Error::Usage(_) => 2,
        Error::Precondition(_) => 3,
        Error::TipMoved(_) => 4,
        Error::Pushed(_) => 5,
        Error::Nonconforming(_) => 6,
        Error::LlmInvalid(_) => 7,
    }
}

fn run(cli: Cli) -> Result<()> {
    let session = Session {
        start: match &cli.dir {
            Some(d) => d.clone(),
            None => std::env::current_dir()?,
        },
        config: cli.config,
        backend: cli.backend,
    };
    match cli.cmd {
        Cmd::Init { force } => commands::init::run(&session, force),
        Cmd::Plan {
            range,
            out,
            check,
            retag,
        } => commands::plan::run(&session, &range, out, check, retag),
        Cmd::Apply { range, plan, retag } => commands::apply::run(&session, &range, plan, retag),
        Cmd::Export {
            range,
            plan,
            batch,
            offset,
        } => commands::llm::export(&session, &range, plan, batch, offset),
        Cmd::Import { plan, out, reply } => commands::llm::import(&session, plan, out, &reply),
        Cmd::Restore { id, force, prune } => commands::restore::run(&session, id, force, prune),
        Cmd::Hook { cmd } => commands::hook::run(&session, cmd),
    }
}

pub fn main() -> i32 {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("gcma: {e}");
            exit_code(&e)
        }
    }
}
