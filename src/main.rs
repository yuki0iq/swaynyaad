use anyhow::{Context, Result};
use std::{env, process};
use swayipc::Connection;

mod barlistener;
mod opener;

fn main() -> Result<()> {
    let conn = Connection::new().context("Create connection")?;

    let args = env::args().collect::<Vec<_>>();
    match args.get(1).map(|arg| arg.as_ref()) {
        Some("--listener") => barlistener::listen_for_bar(conn)?,
        Some("--opener") => opener::open_bars(conn)?,
        Some(_) => {
            eprintln!("Unknown command-line option supplied");
            process::exit(1);
        }
        None => {
            eprintln!("You are not supposed to run this binary directly");
            process::exit(1);
        }
    }

    Ok(())
}
