use std::{
    net::{IpAddr, SocketAddr},
    process::ExitCode,
};

use anyhow::Result;
use clap::Parser;
use sshx_server::{Server, ServerOptions};
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, info};

/// The sshx server CLI interface.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Specify port to listen on.
    #[clap(long, default_value_t = 8051)]
    port: u16,

    /// Which IP address or network interface to listen on.
    #[clap(long, value_parser, default_value = "::1")]
    listen: IpAddr,

    /// Secret used for signing session tokens.
    #[clap(long, env = "SSHX_SECRET")]
    secret: Option<String>,

    /// Override the origin URL returned by the Open() RPC.
    #[clap(long)]
    override_origin: Option<String>,

    /// URL of the Redis server that stores session data.
    #[clap(long, env = "SSHX_REDIS_URL")]
    redis_url: Option<String>,

    /// Hostname of this server, if running multiple servers.
    #[clap(long)]
    host: Option<String>,

    /// Password that gates private board routes.
    #[clap(long, env = "SSHX_BOARD_PASSWORD")]
    board_password: Option<String>,

    /// Path to the file containing the active oracle session URL.
    #[clap(long, env = "SSHX_ORACLE_URL_FILE")]
    oracle_url_file: Option<String>,

    /// Path to the directory containing static assets.
    #[clap(long, default_value = "build")]
    static_dir: String,

    /// Directory for durable board persistence (boards survive restarts).
    #[clap(long, env = "SSHX_PERSIST_DIR")]
    persist_dir: Option<String>,
}

#[tokio::main]
async fn start(args: Args) -> Result<()> {
    let addr = SocketAddr::new(args.listen, args.port);

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    let mut options = ServerOptions::default();
    options.secret = args.secret;
    options.override_origin = args.override_origin;
    options.redis_url = args.redis_url;
    options.host = args.host;
    options.board_password = args.board_password;
    options.oracle_url_file = args.oracle_url_file;
    options.static_dir = Some(args.static_dir);
    options.persist_dir = args.persist_dir;

    let server = Server::new(options).await?;

    let serve_task = async {
        info!("server listening at {addr}");
        server.bind(&addr).await
    };

    let signals_task = async {
        tokio::select! {
            Some(()) = sigterm.recv() => (),
            Some(()) = sigint.recv() => (),
            else => return Ok(()),
        }
        info!("gracefully shutting down...");
        server.shutdown();
        Ok(())
    };

    tokio::try_join!(serve_task, signals_task)?;
    Ok(())
}

fn main() -> ExitCode {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or("info".into()))
        .with_writer(std::io::stderr)
        .init();

    match start(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err:?}");
            ExitCode::FAILURE
        }
    }
}
