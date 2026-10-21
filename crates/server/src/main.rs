mod accounts;
mod activity;
mod adapters;
mod agent;
mod agent_canvas;
mod agent_config;
mod agent_sessions;
#[cfg(test)]
mod api_tests;
mod app;
mod balances;
mod bridge;
mod canvas;
mod chat_adapter;
#[cfg(test)]
mod chat_client_tests;
#[cfg(test)]
mod chat_fixture;
mod checkouts;
mod claude_chat;
mod cli;
mod clients;
mod codex_chat;
mod conversations;
mod daemon;
mod directories;
mod embedded;
mod environment;
mod file_routes;
mod file_search;
mod file_watch;
mod git_ops;
mod git_routes;
mod host_grants;
mod host_routes;
mod installation;
mod launch;
mod layout_routes;
mod mcp;
mod media;
mod model_catalog;
mod model_limits;
mod model_probe;
mod native_chat;
mod native_config;
mod native_history;
mod paths;
mod pi_chat;
mod preferences;
mod price_list;
mod profile_routes;
mod providers;
mod pty_routes;
mod resources;
mod resume;
mod secrets;
mod security;
mod sensors;
mod session_routes;
mod settings;
mod tailscale;
mod update;
mod usage;
mod workspace_io;
mod workspace_routes;

use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, Request, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashSet, env, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tendly_domain::{
    EndpointProfile, InteractionMode, ProviderKind, Session, SessionId, SessionStatus, Workspace,
    WorkspaceId,
};
use tendly_persistence::Store;
use tendly_runtime::{RuntimeEvent, RuntimeManager, RuntimeSession};
#[cfg(test)]
use tokio::net::TcpListener;
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
};
use uuid::Uuid;

use app::*;
use cli::*;
use file_routes::*;
use layout_routes::*;
use pty_routes::*;
use session_routes::*;
use workspace_routes::*;

/// Print failures as text and exit non-zero.
///
/// Returning the error from `main` formats it with `Debug`, which quotes the
/// whole message and prints its newlines as `\n` — so a startup failure that
/// carries a log tail arrived as one unreadable line. A supervisor also needs
/// the non-zero status to tell a failed start from a successful one.
fn main() {
    // The configuration file is read before the runtime is built, because
    // seeding the environment from it is only sound while this is the only
    // thread — and a runtime is what ends that.
    if let Err(error) = apply_settings() {
        eprintln!("{error}");
        std::process::exit(1);
    }
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    runtime.block_on(async {
        if let Err(error) = run().await {
            eprintln!("{error}");
            std::process::exit(1);
        }
    });
    // Stopped for an update under a service manager: exit as a failure so
    // `Restart=on-failure` starts the new binary too.
    if update::restart_requested() {
        std::process::exit(update::RESTART_EXIT_CODE);
    }
}

/// Let the file say what the environment has not.
///
/// A variable that is already set wins, because whoever set it did so later and
/// more deliberately than whoever wrote the file. A state directory that cannot
/// be resolved is not reported here: `run` says the same thing better, and
/// saying it twice from two places would be worse than once.
fn apply_settings() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let Ok(state_dir) = installation::state_directory() else {
        return Ok(());
    };
    let lan = env::args().any(|argument| argument == "--lan");
    if env::args().any(|argument| argument == "--tailscale") {
        // Sound here and nowhere later: no other thread exists yet.
        unsafe { env::set_var("TENDLY_TAILSCALE", "1") };
    }
    for (key, value) in settings::pairs(&state_dir, lan)? {
        if env::var_os(&key).is_none() {
            // Sound here and nowhere later: no other thread exists yet.
            unsafe { env::set_var(&key, value) };
        }
    }
    Ok(())
}

async fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Flags may precede or follow the command, so the command is the first
    // argument that is not one. `tendly --lan` is a start with a flag, not a
    // request to run something called `--lan`.
    let action = env::args()
        .skip(1)
        .find(|argument| !argument.starts_with('-'))
        .unwrap_or_else(|| {
            if env::args().any(|argument| matches!(argument.as_str(), "--help" | "-h")) {
                "--help".into()
            } else if env::args().any(|argument| matches!(argument.as_str(), "--version" | "-V")) {
                "--version".into()
            } else {
                "start".into()
            }
        });
    if matches!(action.as_str(), "--help" | "-h" | "help") {
        println!("{HELP}");
        return Ok(());
    }
    if matches!(action.as_str(), "--version" | "-V" | "version") {
        println!("tendly {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // A relay to the running server: no state directory, database or port.
    // Its stdout is the protocol, so nothing else may print there.
    if action == "mcp" {
        return mcp::serve().await;
    }
    // Replaces the install, not the running gateway: no state directory needed.
    if action == "update" {
        return update::command(env::args().any(|argument| argument == "--check")).await;
    }
    // Asks the running server for a session's launch and runs it here, in this
    // terminal: no state directory or database of its own.
    if action == "resume" {
        return resume::command(env::args().skip(2).collect()).await;
    }
    if !matches!(
        action.as_str(),
        "serve" | "init" | "start" | "stop" | "restart" | "status" | "logs"
    ) {
        return Err(format!("Unknown command {action:?}; use --help").into());
    }
    if action == "serve" {
        // Before an update can replace the file this path names.
        update::remember_executable();
        tracing_subscriber::fmt()
            .with_env_filter(env::var("RUST_LOG").unwrap_or_else(|_| "tendly=info".into()))
            .init();
    }
    let state_dir =
        providers::private_dir(&installation::state_directory()?).map_err(|e| e.message)?;
    let database = env::var("TENDLY_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|_| state_dir.join("tendly.db"));
    if action == "init" {
        let _database = installation::open_database(&state_dir, &database)?;
        println!(
            "Initialized Tendly at {}\nDatabase: {}\nProjects and existing client credentials were not moved.",
            state_dir.display(),
            database.display()
        );
        return Ok(());
    }
    // `--lan` is the whole configuration for reaching this from another machine:
    // the bind, the token and the addresses to answer for all follow from it.
    let lan = env::args().any(|argument| argument == "--lan");
    let address: SocketAddr = match env::var("TENDLY_ADDR") {
        Ok(value) => value.parse()?,
        Err(_) if lan => format!(
            "0.0.0.0:{}",
            DEFAULT_ADDRESS.rsplit(':').next().unwrap_or("")
        )
        .parse()?,
        Err(_) => DEFAULT_ADDRESS.parse()?,
    };
    match action.as_str() {
        "stop" => return gateway_stop(&state_dir).await,
        "status" => return gateway_status(&state_dir, address).await,
        "logs" => return Ok(daemon::print_log(&state_dir, 80)?),
        "start" | "restart" => {
            if action == "restart" {
                gateway_stop(&state_dir).await?;
            }
            return gateway_start(&state_dir, address).await;
        }
        _ => {}
    }
    // Validate configuration and reserve both the port and database ownership
    // before any startup reconciliation can change existing session records.
    let token = security::resolve_token(&state_dir, !address.ip().is_loopback())?;
    // The tailnet always asks for a token, so there is one even on loopback,
    // and switching the tailnet on later needs no restart.
    let access_token = security::resolve_token(&state_dir, true)?;
    let security = security::Security::new(address, token)?
        .with_access_token(access_token)
        .with_session_secret(&security::resolve_session_secret(&state_dir)?);
    let browse_roots = directories::roots()?;
    let workspace_roots = directories::workspace_roots(&browse_roots)?;
    let installation::PreparedServer {
        listener,
        database: locked_database,
    } = installation::prepare_server(address, &state_dir, &database).await?;
    let store = locked_database.store.clone();
    // Where `tailscale serve` forwards to: a port of its own, so whatever
    // arrives on it is known to come from the tailnet.
    let tailnet_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let tailnet = tailscale::Tailnet::new(&state_dir, tailnet_listener.local_addr()?.port());
    let claude_bin = clients::program(&state_dir, &tendly_domain::ProviderKind::ClaudeCode);
    let help = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new(claude_bin)
            .arg("--help")
            .kill_on_drop(true)
            .output(),
    )
    .await;
    let claude_manual_mode = matches!(help,Ok(Ok(ref output)) if String::from_utf8_lossy(&output.stdout).contains("\"manual\""));
    // Before anything resolves a bridge path: a single-file release carries the
    // Node modules inside it and has to place them first.
    installation::place_native_bridge(&state_dir)?;
    let state = AppState {
        store,
        runtime: RuntimeManager::new(),
        native_bridge: installation::native_bridge(&state_dir),
        #[cfg(test)]
        chat_fixture: None,
        chats: conversations::ChatManager::default(),
        watches: file_watch::FileWatches::default(),
        accounts: accounts::AccountManager::new(),
        state_dir,
        browse_roots,
        host_grants: host_grants::HostGrants::default(),
        workspace_roots,
        native_sources: native_history::sources(),
        security,
        tailnet,
        claude_manual_mode,
        operations: Arc::new(tokio::sync::Mutex::new(())),
        agents: agent::AgentRegistry::default(),
        activity: activity::Activity::default(),
        metrics: open_metrics(&database),
    };
    usage::warm(state.clone());
    model_catalog::warm(state.clone());
    price_list::spawn_refresh(state.state_dir.clone());
    if let Some(metrics) = state.metrics.clone() {
        resources::spawn_recorder(state.clone(), metrics);
    }
    let app = router(state.clone());
    // Reconcile only after startup has passed all configuration, binding and
    // ownership checks. `locked_database` stays alive through graceful shutdown.
    state.store.reconcile_after_restart()?;
    tracing::info!(%address,db=%database.display(),"Tendly listening (trusted single-user host mode)");
    let tailnet_server = tokio::spawn(
        axum::serve(
            tailnet_listener,
            app.clone()
                .layer(axum::Extension(security::ViaTailnet))
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .into_future(),
    );
    state.tailnet.start();
    // The peer address lets a few routes answer only this machine (`launch-plan`).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM");
            tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
        state.tailnet.shutdown().await;
        state.runtime.shutdown().await;
        state.chats.shutdown().await;
        state.accounts.shutdown().await;
    })
    .await?;
    tailnet_server.abort();
    Ok(())
}

/// Beside the main database, so a test or custom `TENDLY_DB` gets its own.
/// History is disposable: if it cannot be opened the server runs without it.
fn open_metrics(
    database: &std::path::Path,
) -> Option<Arc<tendly_persistence::metrics::MetricsStore>> {
    let path = database.with_file_name("tendly-metrics.db");
    match tendly_persistence::metrics::MetricsStore::open(&path) {
        Ok(store) => Some(Arc::new(store)),
        Err(error) => {
            tracing::warn!(%error, path=%path.display(), "resource history disabled");
            None
        }
    }
}

fn router(state: AppState) -> Router {
    // A real build on disk wins, so a development tree or an installed layout
    // serves the files it just wrote. Otherwise the build that travels inside
    // this binary is served from memory. Existence decides it rather than the
    // override merely being set, so a path that points nowhere degrades to the
    // embedded copy instead of serving nothing at all.
    let web = installation::web_directory(&state.state_dir);
    let from_disk = web.join("index.html").exists();
    if !from_disk && !embedded::has_web() {
        tracing::warn!("no web assets on disk or in this binary; run the web build");
    }
    Router::new()
        .merge(accounts::routes())
        .merge(clients::routes())
        .merge(resources::routes())
        .merge(model_limits::routes())
        .merge(usage::routes())
        .merge(host_grants::routes())
        .merge(checkouts::routes())
        .merge(git_ops::routes())
        .merge(preferences::routes())
        .merge(secrets::routes())
        .merge(agent::routes())
        .merge(activity::routes())
        .merge(conversations::routes())
        .merge(file_watch::routes())
        .merge(update::routes())
        .merge(tailscale::routes())
        .merge(host_routes::routes())
        .merge(workspace_routes::routes())
        .merge(session_routes::routes())
        .merge(pty_routes::routes())
        .merge(profile_routes::routes())
        .merge(balances::routes())
        .merge(file_routes::routes())
        .merge(git_routes::routes())
        .merge(layout_routes::routes())
        .route("/api/health", get(health))
        .route("/api/auth", get(security::status).post(security::login))
        .route(
            "/api/canvas/layout",
            get(canvas::get_layout).put(canvas::save_layout),
        )
        .route(
            "/api/{*path}",
            get(|| async { ApiError::missing("API route") }),
        )
        .fallback_service(if from_disk {
            axum::routing::any_service(
                ServeDir::new(&web).not_found_service(ServeFile::new(web.join("index.html"))),
            )
        } else {
            axum::routing::any(|uri: axum::http::Uri| async move { embedded::serve_web(&uri) })
        })
        .layer(DefaultBodyLimit::max(3 * 1024 * 1024))
        .layer(CompressionLayer::new().compress_when(compressible))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security::guard,
        ))
        .with_state(state)
}

/// Text worth compressing: conversation snapshots and other JSON, and the
/// web bundle. Media, downloads and partial responses go as they are -- they
/// are already compressed or are read by byte range -- and so does anything
/// small enough that compressing it costs more than it saves.
fn compressible(
    status: axum::http::StatusCode,
    _: axum::http::Version,
    headers: &axum::http::HeaderMap,
    _: &axum::http::Extensions,
) -> bool {
    if status == axum::http::StatusCode::PARTIAL_CONTENT {
        return false;
    }
    let small = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|length| length < 1024);
    let kind = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    !small
        && (kind.starts_with("application/json")
            || kind.starts_with("text/html")
            || kind.starts_with("text/css")
            || kind.starts_with("text/javascript")
            || kind.starts_with("application/javascript")
            || kind.starts_with("image/svg+xml"))
}

async fn health() -> Json<Value> {
    Json(
        json!({"ok":true,"service":"tendly-server","platform":env::consts::OS,"mode":"trusted-single-user","api_version":2,"capabilities":["shared_canvas","native_configurations","native_history","host_directories","endpoint_models","model_probe",
            "model_probe_plan","session_environment","structured_chat","official_accounts","session_configuration","session_archive","account_import_native","agent_clients","ephemeral_sessions","session_model","workspace_file_search","session_terminal_escape","stored_secrets","custom_balances","agent_tools","self_update","resource_monitoring","tailnet"],"instance_label":env::var("TENDLY_INSTANCE_LABEL").ok(),"version":env!("CARGO_PKG_VERSION")}),
    )
}

#[cfg(test)]
mod router_tests {
    use super::*;
    use axum::http::{Extensions, HeaderMap, HeaderValue, StatusCode as Status, Version, header};

    fn compressed(status: Status, kind: &str, length: Option<u64>) -> bool {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(kind).unwrap());
        if let Some(length) = length {
            headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
        }
        compressible(status, Version::HTTP_11, &headers, &Extensions::new())
    }

    #[test]
    fn text_is_compressed_and_media_ranges_and_small_bodies_are_not() {
        assert!(compressed(Status::OK, "application/json", Some(50_000)));
        assert!(
            compressed(Status::OK, "text/javascript; charset=utf-8", None),
            "a stream of unknown length"
        );
        assert!(compressed(Status::OK, "image/svg+xml", Some(4096)));
        assert!(
            !compressed(Status::OK, "image/png", Some(50_000)),
            "already compressed"
        );
        assert!(
            !compressed(Status::OK, "application/json", Some(1023)),
            "too small to be worth it"
        );
        assert!(
            !compressed(Status::PARTIAL_CONTENT, "application/json", Some(50_000)),
            "a byte range"
        );
    }
}
