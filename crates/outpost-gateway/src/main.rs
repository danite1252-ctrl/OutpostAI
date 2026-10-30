use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use outpost_gateway::config::{default_data_dir, DEFAULT_OLLAMA_URL};
use outpost_gateway::{AppState, Config};

/// Outpost local API gateway.
#[derive(Parser)]
#[command(name = "outpost-gateway", version, about)]
struct Args {
    /// Folder for the database and account files.
    #[arg(long, env = "OUTPOST_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// Address to listen on, e.g. 0.0.0.0:7420 for other devices. Without it, Outpost
    /// uses the "Allow other devices on my network" setting (127.0.0.1:7420 when off).
    #[arg(long, env = "OUTPOST_BIND")]
    bind: Option<SocketAddr>,

    /// Ollama's address.
    #[arg(long, env = "OUTPOST_OLLAMA_URL", default_value = DEFAULT_OLLAMA_URL)]
    ollama_url: String,

    /// Test run: make a dataset for the owner from this folder of pictures, caption
    /// it, queue its training, then serve as usual.
    #[arg(long, value_name = "FOLDER")]
    queue_training: Option<PathBuf>,

    /// The test character's name (with --queue-training).
    #[arg(long, default_value = "Character")]
    train_name: String,

    /// Its trigger word (with --queue-training); made from the name if left out.
    #[arg(long)]
    train_trigger: Option<String>,

    /// Training steps (with --queue-training).
    #[arg(long, default_value_t = 2000)]
    train_steps: u32,

    /// Act as a local MCP server for apps like Claude Desktop: read MCP messages on
    /// stdin and pass them to the running Outpost (needs --key or OUTPOST_KEY).
    #[arg(long)]
    mcp_stdio: bool,

    /// An Outpost app key, for --mcp-stdio.
    #[arg(long, env = "OUTPOST_KEY", hide_env_values = true)]
    key: Option<String>,

    /// Print the OpenAPI spec as JSON and exit.
    #[arg(long)]
    print_openapi: bool,

    /// Run as a server with no screen (a home server): network access is on unless the
    /// owner turns it off, so other devices can set it up and use it.
    #[arg(long, env = "OUTPOST_SERVER")]
    server: bool,

    /// Locked out? Make a PIN-reset invite for this account (by name), print its code and
    /// check words, then exit. Use it from the desktop or phone app like a pairing code:
    /// you pick a new PIN there. Works once, for 24 hours.
    #[arg(long, value_name = "NAME")]
    reset_pin: Option<String>,

    /// Show the one-time setup code for creating the first owner from another device
    /// (only while Outpost has no owner), then exit.
    #[arg(long)]
    setup_code: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.print_openapi {
        println!("{}", outpost_gateway::openapi::spec_json());
        return Ok(());
    }

    if args.setup_code {
        let data_dir = args.data_dir.clone().unwrap_or_else(default_data_dir);
        match std::fs::read_to_string(outpost_gateway::setup_code::file(&data_dir)) {
            Ok(text) => print!("{text}"),
            Err(_) => println!(
                "There's no setup code: Outpost already has an owner, or it isn't running yet \
                 (start it, wait a few seconds, and try again)."
            ),
        }
        return Ok(());
    }

    if let Some(name) = &args.reset_pin {
        let data_dir = args.data_dir.clone().unwrap_or_else(default_data_dir);
        return reset_pin(&data_dir, name).await;
    }

    if args.mcp_stdio {
        // stdout carries MCP messages only: no logging here.
        let key = args
            .key
            .ok_or_else(|| anyhow::anyhow!("--mcp-stdio needs an app key: --key op_... or OUTPOST_KEY"))?;
        let port = args.bind.map(|b| b.port()).unwrap_or(outpost_gateway::config::DEFAULT_PORT);
        return outpost_gateway::mcp::stdio_bridge(format!("http://127.0.0.1:{port}/mcp"), key).await;
    }

    // Logs to the console and <data>/logs; crash reports to <data>/crash-reports.
    let data_dir = args.data_dir.clone().unwrap_or_else(default_data_dir);
    outpost_gateway::diagnostics::init(
        &data_dir,
        "gateway",
        "outpost_gateway=info,outpost_core=info,outpost_engines=info,tower_http=info",
    );

    let mut config = Config::local_default();
    config.data_dir = args.data_dir.unwrap_or_else(default_data_dir);
    config.ollama_url = args.ollama_url;
    config.server = args.server;
    if let Some(bind) = args.bind {
        config.bind = bind;
    }
    tracing::info!(data_dir = %config.data_dir.display(), "opening Outpost data");
    let mut state = AppState::open(config).await?;
    if args.bind.is_none() {
        state.apply_network_setting().await?;
    }

    let bind = state.config.bind;
    if !bind.ip().is_loopback() {
        tracing::info!(
            %bind,
            secure_port = outpost_gateway::secure_port(bind.port()),
            "listening on the network: paired devices use the encrypted port; app keys work on the plain one, so use a network you trust"
        );
    }

    if let Some(folder) = args.queue_training {
        tokio::spawn(outpost_gateway::train::queue_from_folder(
            state.clone(),
            folder,
            args.train_name,
            args.train_trigger,
            args.train_steps,
        ));
    }
    let result = outpost_gateway::serve_state(state).await;
    outpost_gateway::diagnostics::clean_exit();
    result
}

/// `--reset-pin NAME`: a reset invite made on the server itself (reading the data folder
/// means having the server's own login, which is the proof needed).
async fn reset_pin(data_dir: &std::path::Path, name: &str) -> anyhow::Result<()> {
    let db = data_dir.join("outpost.db");
    if !db.is_file() {
        anyhow::bail!("There's no Outpost data in {}.", data_dir.display());
    }
    let pool = outpost_core::db::open(&db).await?;
    let accounts = outpost_core::accounts::list(&pool).await?;
    let Some(account) = accounts.iter().find(|a| a.name.eq_ignore_ascii_case(name.trim())) else {
        let names: Vec<&str> = accounts.iter().map(|a| a.name.as_str()).collect();
        anyhow::bail!("No account called \"{name}\". The accounts are: {}.", names.join(", "));
    };
    let (code, row) = outpost_core::pairing::create_invite(
        &pool,
        &account.id,
        outpost_core::pairing::Invite::Reset { account_id: account.id.clone() },
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    let words = outpost_gateway::tls::load_or_create(data_dir)
        .map(|id| outpost_core::pairing::words(&id.fingerprint).join(" "))
        .unwrap_or_default();
    let host = outpost_gateway::routes::network::lan_ip().map(|ip| ip.to_string()).unwrap_or_else(|| "this server's address".into());
    println!("PIN reset for {}", account.name);
    println!("Code:        {}", code);
    println!("Check words: {words}");
    println!("Valid until: {} (UTC)", outpost_gateway::diagnostics::rfc3339(row.expires_at));
    println!();
    println!("In the Outpost desktop app choose \"Connect to an Outpost server\" (or on a phone, pair by");
    println!("typing the code), connect to {host}, check the words match, enter the code and pick a new PIN.");
    println!("The new PIN signs {} out on every other device.", account.name);
    Ok(())
}
