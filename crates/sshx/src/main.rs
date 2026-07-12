use std::process::ExitCode;

use ansi_term::Color::{Cyan, Fixed, Green};
use anyhow::Result;
use clap::Parser;
use sshx::{controller::Controller, runner::Runner, terminal::get_default_shell};
use tokio::signal;
use tracing::error;

/// A secure web-based, collaborative terminal.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Address of the remote sshx server.
    #[clap(long, default_value = "https://sshx.io", env = "SSHX_SERVER")]
    server: String,

    /// Local shell command to run in the terminal.
    #[clap(long)]
    shell: Option<String>,

    /// Quiet mode, only prints the URL to stdout.
    #[clap(short, long)]
    quiet: bool,

    /// Session name displayed in the title (defaults to user@hostname).
    #[clap(long)]
    name: Option<String>,

    /// Enable read-only access mode - generates separate URLs for viewers and
    /// editors.
    #[clap(long)]
    enable_readers: bool,

    /// Join an existing session (by name) as an additional backend, instead
    /// of creating a new one. Requires --join-token and --encryption-key
    /// (get both from the primary backend's own output).
    #[clap(long, requires_all = ["join_token", "encryption_key"])]
    join: Option<String>,

    /// Join-authorization token for --join, printed by the primary backend.
    #[clap(long)]
    join_token: Option<String>,

    /// Encryption key to join with — MUST match the session's own key
    /// (copy it from the primary's write/read URL fragment, the part after
    /// '#'). A mismatched key is rejected by the server rather than
    /// producing undecryptable garbage.
    #[clap(long)]
    encryption_key: Option<String>,

    /// Human-readable label for this backend when joining (defaults to
    /// user@hostname, same as --name).
    #[clap(long)]
    backend_name: Option<String>,

    /// Connector bearer token identifying the account that owns this backend
    /// (VR5). Set by the oracle connector in remote mode; the server rejects
    /// the join if it's present but unrecognized. Omit for local/legacy joins.
    #[clap(long, env = "CONNECTOR_TOKEN")]
    connector_token: Option<String>,
}

fn print_greeting(shell: &str, controller: &Controller) {
    let version_str = match option_env!("CARGO_PKG_VERSION") {
        Some(version) => format!("v{version}"),
        None => String::from("[dev]"),
    };
    if let Some(write_url) = controller.write_url() {
        println!(
            r#"
  {sshx} {version}

  {arr}  Read-only link: {link_v}
  {arr}  Writable link:  {link_e}
  {arr}  Shell:          {shell_v}
"#,
            sshx = Green.bold().paint("sshx"),
            version = Green.paint(&version_str),
            arr = Green.paint("➜"),
            link_v = Cyan.underline().paint(controller.url()),
            link_e = Cyan.underline().paint(write_url),
            shell_v = Fixed(8).paint(shell),
        );
    } else {
        println!(
            r#"
  {sshx} {version}

  {arr}  Link:  {link_v}
  {arr}  Shell: {shell_v}
"#,
            sshx = Green.bold().paint("sshx"),
            version = Green.paint(&version_str),
            arr = Green.paint("➜"),
            link_v = Cyan.underline().paint(controller.url()),
            shell_v = Fixed(8).paint(shell),
        );
    }
    if let Some(join_token) = controller.join_token() {
        println!(
            "  {arr}  To join from another node:\n      sshx --server {server} --join {name} \\\n        --join-token {token} --encryption-key {key}\n",
            arr = Green.paint("➜"),
            server = controller.origin(),
            name = controller.name(),
            token = join_token,
            key = controller.encryption_key(),
        );
    }
}

#[tokio::main]
async fn start(args: Args) -> Result<()> {
    let shell = match args.shell {
        Some(shell) => shell,
        None => get_default_shell().await,
    };

    let name = args.name.clone().unwrap_or_else(|| {
        let mut name = whoami::username();
        if let Ok(host) = whoami::fallible::hostname() {
            // Trim domain information like .lan or .local
            let host = host.split('.').next().unwrap_or(&host);
            name += "@";
            name += host;
        }
        name
    });

    let runner = Runner::Shell(shell.clone());
    let mut controller = match args.join {
        Some(join_name) => {
            let backend_name = args.backend_name.unwrap_or_else(|| name.clone());
            Controller::join(
                &args.server,
                &join_name,
                args.join_token.as_deref().expect("clap requires_all"),
                args.encryption_key.as_deref().expect("clap requires_all"),
                &backend_name,
                args.connector_token.as_deref(),
                runner,
            )
            .await?
        }
        None => Controller::new(&args.server, &name, runner, args.enable_readers).await?,
    };
    if args.quiet {
        if let Some(write_url) = controller.write_url() {
            println!("{}", write_url);
        } else {
            println!("{}", controller.url());
        }
    } else {
        print_greeting(&shell, &controller);
    }

    let exit_signal = signal::ctrl_c();
    tokio::pin!(exit_signal);
    tokio::select! {
        _ = controller.run() => unreachable!(),
        Ok(()) = &mut exit_signal => (),
    };
    controller.close().await?;

    Ok(())
}

fn main() -> ExitCode {
    let args = Args::parse();

    let default_level = if args.quiet { "error" } else { "info" };

    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or(default_level.into()))
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
