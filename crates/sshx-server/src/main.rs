use std::{
    net::{IpAddr, SocketAddr},
    process::ExitCode,
};

use anyhow::{anyhow, Result};
use clap::Parser;
use sshx_server::state::accounts::AccountsDb;
use sshx_server::state::disk::StorageDisk;
use sshx_server::{auth, Server, ServerOptions};
use tokio::signal::unix::{signal, SignalKind};
use tracing::{error, info};

/// The sshx server CLI interface.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Optional administrative subcommand. With none, runs the server.
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    serve: ServeArgs,
}

/// Flags for running the server (the default, subcommand-less invocation).
#[derive(clap::Args, Debug)]
struct ServeArgs {
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

    /// Allowed browser Origin for cross-origin-sensitive requests, e.g.
    /// `https://board.off-scrn.com`. Unset disables the check (same-origin dev).
    #[clap(long, env = "SSHX_ALLOWED_ORIGIN")]
    allowed_origin: Option<String>,

    /// Emit session cookies without the `Secure` attribute — plain-HTTP
    /// localhost dev only. Never set this in production.
    #[clap(long)]
    insecure_cookies: bool,

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

/// Administrative subcommands (Vision Round 5 F0). Each reuses the same
/// [`AccountsDb`] layer as the server, so there is exactly one implementation
/// of the accounts schema.
#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Manage account invite codes.
    Invite {
        #[command(subcommand)]
        action: InviteAction,
    },
    /// One-time F0 migration: bootstrap the owner account and record ownership
    /// of every board already in the persist dir. Idempotent.
    MigrateVr5(MigrateArgs),
}

#[derive(clap::Subcommand, Debug)]
enum InviteAction {
    /// Create a single-use invite code, printing it to stdout.
    Create(InviteCreateArgs),
}

#[derive(clap::Args, Debug)]
struct InviteCreateArgs {
    /// Persist directory holding `accounts.db`.
    #[clap(long, env = "SSHX_PERSIST_DIR")]
    persist_dir: String,

    /// Username of the (existing) account the invite is attributed to.
    #[clap(long, default_value = "louis")]
    created_by: String,
}

#[derive(clap::Args, Debug)]
struct MigrateArgs {
    /// Persist directory holding board snapshots and `accounts.db`.
    #[clap(long, env = "SSHX_PERSIST_DIR")]
    persist_dir: String,

    /// Password for the bootstrapped owner account. Prefer the env var over a
    /// CLI flag so it doesn't land in shell history / the process list.
    #[clap(long, env = "SSHX_MIGRATE_PASSWORD")]
    password: String,

    /// Username for the bootstrapped owner account.
    #[clap(long, default_value = "louis")]
    username: String,
}

#[tokio::main]
async fn run_server(args: ServeArgs) -> Result<()> {
    let addr = SocketAddr::new(args.listen, args.port);

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    // ServerOptions is #[non_exhaustive]; build via Default then assign fields.
    let mut options = ServerOptions::default();
    options.secret = args.secret;
    options.override_origin = args.override_origin;
    options.redis_url = args.redis_url;
    options.host = args.host;
    options.allowed_origin = args.allowed_origin;
    options.insecure_cookies = args.insecure_cookies;
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

#[tokio::main]
async fn run_invite_create(args: InviteCreateArgs) -> Result<()> {
    let db = AccountsDb::new(Some(&args.persist_dir)).await?;
    let account = db
        .account_by_username(&args.created_by)
        .await?
        .ok_or_else(|| {
            anyhow!(
                "no account '{}' — run `migrate-vr5` (or create it) first",
                args.created_by
            )
        })?;
    let code = db.create_invite(&account.id).await?;
    // The code is the machine-readable output on stdout; the human hint goes to
    // stderr so it can be captured cleanly by a script if desired.
    println!("{code}");
    eprintln!("invite created by '{}' — join at /join?code={code}", args.created_by);
    Ok(())
}

#[tokio::main]
async fn run_migrate_vr5(args: MigrateArgs) -> Result<()> {
    let db = AccountsDb::new(Some(&args.persist_dir)).await?;

    // 1. Bootstrap the owner account (bypasses the invite chicken-egg).
    let hash = auth::hash_account_password(&args.password)?;
    let account = db.bootstrap_account(&args.username, &hash).await?;

    // 2. Record ownership of every board already persisted on disk. create_board
    //    is idempotent (INSERT OR IGNORE), so re-running is safe.
    let disk = StorageDisk::new(&args.persist_dir)?;
    let mut count = 0usize;
    for board in disk.list() {
        db.create_board(&board.name, &account.id).await?;
        count += 1;
    }

    eprintln!(
        "migrate-vr5: account '{}' now owns {count} board(s)",
        args.username
    );
    Ok(())
}

fn main() -> ExitCode {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or("info".into()))
        .with_writer(std::io::stderr)
        .init();

    let result = match args.command {
        None => run_server(args.serve),
        Some(Command::Invite {
            action: InviteAction::Create(a),
        }) => run_invite_create(a),
        Some(Command::MigrateVr5(a)) => run_migrate_vr5(a),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err:?}");
            ExitCode::FAILURE
        }
    }
}
