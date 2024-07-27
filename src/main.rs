#![feature(let_chains)]

use anyhow::{bail, Context, Result};
use std::env;
use swayipc_async::Connection;

mod barlistener;
mod opener;

async fn async_main() -> Result<()> {
    let conn = Connection::new()
        .await
        .context("create initial connection")?;

    let args = env::args().collect::<Vec<_>>();
    match args.get(1).map(|arg| arg.as_ref()) {
        Some("--listener") => barlistener::listen_for_bar(conn).await?,
        Some("--opener") => opener::open_bars(conn).await?,
        Some(_) => bail!("Unknown command-line option supplied"),
        None => bail!("You are not supposed to run this binary directly"),
    }

    Ok(())
}

fn main() -> Result<()> {
    smol::block_on(async_main())
}
