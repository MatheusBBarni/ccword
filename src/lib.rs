//! Local word completion for a Claude Code terminal session.
//!
//! The binary is a PTY wrapper. Hints are drawn on the real terminal and are
//! not written to Claude unless the user accepts them.

pub mod adapter;
pub mod apple;
pub mod complete;
pub mod config;
pub mod config_tui;
pub mod ctl;
pub mod debuglog;
pub mod doctor;
pub mod eligibility;
pub mod engine;
pub mod error;
pub mod ghost;
pub mod hook;
pub mod keys;
pub mod learn;
pub mod ngram;
pub mod paths;
pub mod resolve;
pub mod session;
pub mod store;
pub mod term;
pub mod tokenize;
pub mod utf16;

pub use error::{Error, Result};

pub fn run() -> Result<i32> {
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.len() == 1 {
        if let Some(command) = args[0].to_str().and_then(ctl::flag_command) {
            return ctl::run(&[command.into()]);
        }
    }
    match args.first().and_then(|arg| arg.to_str()) {
        Some("ctl") => {
            args.remove(0);
            ctl::run(&args)
        }
        Some("hook") => hook::run_hook(),
        _ => session::launch(&args),
    }
}
