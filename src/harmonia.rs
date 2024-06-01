//! Synchronized player for laptop orchestra
//!
//! Harmonia is a music player indended for the use in distributed system of laptop orchestra,
//! where orchestra members want to synchronize their performance, especially in the context of
//! indirect control like MIDI files, algorythmic music or audio files.
//!
//! Harmonia consists of three main components:
//!
//! * Synchronization layer, based on [Ableton Link][rusty_link] and it's extension [linky_groups]
//! * Simple user interface, rendered in the browser, build with [axum], [htmx], [maud]
//! * Audio engine, coordinating the execution of played music [audio_engine]
//!
//! [htmx]: https://htmx.org/
//! [audio_engine]: crate::audio_engine

use anyhow::Context;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        ConnectInfo, State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::{delete, get, post, put},
    Router, TypedHeader,
};
use clap::Parser;
use midir::{MidiOutput, MidiOutputPort};
use rusty_link::AblLink;
use std::{
    collections::HashMap,
    io::BufReader,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    process::ExitCode,
    sync::{Arc, RwLock},
    time::Duration,
};
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

mod audio_engine;
use audio_engine::AudioEngine;
mod version;
use version::Version;
mod block;
mod handlers;
mod public;


/// Filename under which Harmonia stores blocks, user info and other metadata
const STATE_PATH: &str = "harmonia_state.bson";

/// All MIDI output connections that user may use
pub struct MidiConnection {
    /// Currently known [MidiOutputPort]s
    pub ports: Vec<MidiOutputPort>,
}

impl Default for MidiConnection {
    fn default() -> Self {
        // TODO: Is it valid to create a new MidiOutput per use? Maybe we should create only one
        // MidiOutput port per application.
        let out = MidiOutput::new("harmonia").unwrap();

        Self { ports: out.ports() }
    }
}

impl MidiConnection {
    /// Update list of currently known [MidiOutputPort]s
    pub fn refresh(&mut self) {
        // TODO: Is it valid to create a new MidiOutput per use? Maybe we should create only one
        // MidiOutput port per application.
        let out = MidiOutput::new("harmonia").unwrap();
        self.ports = out.ports();
    }
}

/// Shared state between major modules of Harmonia
///
/// Collection of references to particular state, each behind it's own synchronization mechanism to
/// allow as concurrent access as possible.
pub struct AppState {
    /// List of all `blocks` (representations of anything that Harmonia can "play")
    ///
    /// Indexed by unique identifier that is created based on the type of block and hash of the
    /// content. Main source for information both for UI and [audio_engine]. Hold exclusive locks
    /// as little as possible.
    ///
    /// [audio_engine]: crate::audio_engine
    pub blocks: RwLock<HashMap<String, block::Block>>,

    /// List of all MIDI connections
    pub connection: RwLock<MidiConnection>,

    /// Reference to [Ableton Link][rusty_link], base for synchronization mechanism.
    ///
    /// Utilised by [linky_groups] for synchronized start and [audio_engine] as a source of time
    pub link: Arc<AblLink>,

    /// [audio_engine] shared state that allows to send commands from UI to engine (and back)
    pub audio_engine: RwLock<AudioEngine>,


    // TODO: Be better
    /// Identifier of currently playing block, used in UI
    pub currently_playing_uuid: RwLock<Option<String>>,

    /// Progress on currently playing block in form `(done, len)`
    ///
    /// For infinite blocks (like [block::Content::SharedMemory]) `(0, 0)`
    pub current_playing_progress: RwLock<(usize, usize)>,

    /// Port on which to serve HTTP UI
    pub port: u16,

    /// [linky_groups] synchronization mechanism
    pub groups: Option<linky_groups::Groups>,
}

/// Path to the cache location, based on OS convention
///
/// Should conform to XDG_BASE_DIRECTORIES or any other particular operating system standard for
/// cache storage.
fn cache_path() -> PathBuf {
    let path = dirs::cache_dir()
        .expect("documentation states that this function should work on all platforms")
        .join("harmonia");
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// Path to the logs location
///
/// To simplify experience for non-technical users it is stored in the same location as cache,
/// which makes only 1 directory entry when user needs to report bugs.
fn log_path() -> PathBuf {
    cache_path()
}

impl AppState {
    /// Crate new [AppState] (once per Harmonia instance)
    ///
    /// Creates Ableton Link session and [linky_groups] session
    fn new(port: u16) -> Self {
        let link = Arc::new(AblLink::new(120.));
        Self {
            blocks: Default::default(),
            connection: Default::default(),
            link: link.clone(),
            audio_engine: Default::default(),
            currently_playing_uuid: Default::default(),
            current_playing_progress: Default::default(),
            port,
            groups: Some(linky_groups::listen(link)),
        }
    }

    /// Load stored [AppState] from [STATE_PATH]
    fn recollect_previous_blocks(&self) -> Result<(), anyhow::Error> {
        let path = cache_path().join(STATE_PATH);
        let file = std::fs::File::open(path).context("opening state file")?;

        let new_sources: HashMap<String, block::Block> =
            bson::from_reader(BufReader::new(file)).context("reading bson file")?;
        let mut sources = self.blocks.write().unwrap();
        sources.extend(new_sources);

        Ok(())
    }

    /// Store [AppState] in [STATE_PATH]
    fn remember_current_blocks(&self) -> Result<(), anyhow::Error> {
        let sources = self.blocks.read().unwrap();
        let path = cache_path().join(STATE_PATH);
        std::fs::write(path, bson::to_vec(&*sources).context("sources to vec")?)
            .context("saving sources to file")?;
        Ok(())
    }
}

#[derive(Parser, Debug)]
#[command(version = format!("{}", Version::default()))]
/// Harmonia is a synchronized MIDI and music player for laptop orchestra
struct Args {
    /// Don't start link connection
    #[arg(long)]
    disable_link: bool,

    /// Open UI in default browser
    #[arg(long)]
    open: bool,

    /// IP for UI
    #[arg(short, long, default_value_t = String::from("0.0.0.0"))]
    ip: String,

    /// Port for UI
    #[arg(short, long, default_value_t = 8080)]
    port: u16,
}

/// Initialize Harmonia logging system
///
/// Harmonia logs all the events inside log files, each file timestamped by day.
fn setup_logging_system() -> tracing_appender::non_blocking::WorkerGuard {
    let log_file_appender = tracing_appender::rolling::daily(log_path(), "logs");
    let (log_file_appender, guard) = tracing_appender::non_blocking(log_file_appender);

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "harmonia=info,linky_groups=info,linky_groups::net=info".into()
            }),
        )
        .with(
            tracing_subscriber::fmt::layer().and_then(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(log_file_appender),
            ),
        )
        .init();
    guard
}

/// Initialize Harmonia application
///
/// Setup all synchronization mechanisms, HTTP server, recollect stored [AppState] from [cache],
/// enable [logging mechanism][logs], mount [handlers] to HTTP serer, listen at provided HTTP port
/// and ensure proper cleanup on exit.
///
/// [cache]: cache_path()
/// [logs]: setup_logging_system()
#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let _guard = setup_logging_system();

    info!("starting up version {}", Version::default());

    let app_state = Arc::new(AppState::new(args.port));
    if let Err(err) = app_state.recollect_previous_blocks() {
        warn!("trying to recollect previous sources: {err:#}")
    } else {
        info!(
            "recollected {count} blocks",
            count = app_state.blocks.read().unwrap().len()
        )
    }

    app_state.audio_engine.write().unwrap().state = Arc::downgrade(&app_state);
    app_state.link.enable(!args.disable_link);
    info!(
        "link {}",
        if args.disable_link {
            "not active"
        } else {
            "active"
        }
    );

    // Conventions:
    //   Paths begining with /api/ are meant for JavaScript
    //   Others are for HTML / HTMLX consumption
    let app = Router::new()
        .route(
            "/api/link-status-websocket",
            get(link_status_websocket_handler),
        )
        .route("/blocks/midi", put(handlers::add_new_midi_source_block))
        .route(
            "/blocks/shared_memory",
            put(handlers::add_new_shered_memory_block),
        )
        .route("/blocks/:uuid", delete(handlers::remove_block))
        .route("/blocks/:uuid", get(handlers::download_block_content))
        .route("/blocks/play/:uuid", post(handlers::play))
        .route(
            "/blocks/midi/set-port/:uuid",
            post(handlers::set_port_for_midi),
        )
        .route("/blocks/set-group/:uuid", post(handlers::set_group))
        .route("/blocks/set-keybind/:uuid", post(handlers::set_keybind))
        .route("/interrupt", post(handlers::interrupt))
        .route("/", get(handlers::index))
        .route("/htmx.min.js", public::static_response!(get, "htmx.min.js"))
        .route("/index.js", public::static_response!(get, "index.js"))
        .route("/index.css", public::static_response!(get, "index.css"))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .with_state(app_state.clone());

    let ip: IpAddr = args.ip.parse().unwrap();

    let addr = SocketAddr::from((ip, args.port));

    let Ok(builder) = axum::Server::try_bind(&addr) else {
        error!("Address already in use at http://{addr}");
        return ExitCode::FAILURE;
    };

    let display_address = if addr.ip().is_unspecified() {
        SocketAddr::new(Ipv4Addr::new(127, 0, 0, 1).into(), addr.port())
    } else {
        addr
    };

    info!("Listening on http://{display_address}");
    let server = builder
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let ctrl_c = async {
                tokio::signal::ctrl_c()
                    .await
                    .expect("failed to install CTRL-C handler -_-")
            };

            #[cfg(unix)]
            let terminate = async {
                use tokio::signal::unix::{signal, SignalKind};
                signal(SignalKind::terminate())
                    .expect("failed to install terminate signal handler -_-")
                    .recv()
                    .await
            };

            #[cfg(not(unix))]
            let terminate = std::future::pending::<()>();

            tokio::select! {
                _ = ctrl_c => {},
                _ = terminate => {},
            }
        });

    if args.open {
        info!("opening UI in default browser");
        open::that_detached(format!("http://{display_address}")).unwrap();
    }

    server.await.unwrap();
    audio_engine::quit(app_state.clone()).await;
    app_state.link.enable(false);
    // TODO: app_state.groups.take().expect("we are first to clean up this field so value should be here").shutdown().await;
    ExitCode::SUCCESS
}

// For expanding this websocket buisness see: https://github.com/tokio-rs/axum/blob/main/examples/websockets/src/main.rs
/// Handler transferring communication from HTTP to WebSockets
async fn link_status_websocket_handler(
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    app_state: State<Arc<AppState>>,
) -> impl IntoResponse {
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        "unknown user agent".to_string()
    };
    info!("websocket connect: addr={addr}, user_agent={user_agent}");
    ws.on_upgrade(move |socket| link_status_websocket_loop(socket, addr, app_state))
}

/// Loop that sends over WebSocket current state of Harmonia
///
/// This usage of WebSockets is mostly intended to not distract HTTP server with constant requests
/// about application state and allow Harmonia to dictate the tempo of changes that is shown in UI.
/// For constantly updating time the second part is not as important as the first one, but in case
/// of more distinct and slow updates (like from [audio_engine]) this mechanism would be perfect
/// (and probably will be included in future release).
async fn link_status_websocket_loop(
    mut socket: WebSocket,
    addr: SocketAddr,
    app_state: State<Arc<AppState>>,
) {
    loop {
        let markup = handlers::runtime_status(app_state.clone()).await;
        if let Err(err) = socket.send(Message::Text(markup.into_string())).await {
            error!("websocket send to {addr} failed: {err}");
            break;
        }
        // TODO: Sleep should be based on BPM to keep in sync with clock as good as possible
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = socket.close().await;
}
