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
        ws::{Message, Utf8Bytes, WebSocket},
        ConnectInfo, State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::{any, delete, get, post, put},
    Router,
};
use axum_extra::TypedHeader;
use clap::Parser;
use maud::html;
use rusty_link::AblLink;
use std::{
    collections::HashMap,
    io::BufReader,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    process::ExitCode,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
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

/// Filename under which Harmonia stores user's nick
const NICK_PATH: &str = "harmonia_nick.txt";

/// All MIDI output connections that user may use
pub struct MidiConnection {
    /// Connection to the MIDI Client
    pub conn: Arc<Mutex<midir::MidiOutput>>,

    /// Currently known [MidiOutputPort]s
    pub ports: Vec<midir::MidiOutputPort>,

    /// Virtual port created by default on unix platforms
    ///
    /// On Linux it isn't necessary needed since it has default MIDI output port from operating
    /// system. On macOS it is required since by default there are no MIDI outputs to use.
    #[cfg(unix)]
    pub virtual_port: Arc<Mutex<midir::MidiOutputConnection>>,
}

impl Default for MidiConnection {
    fn default() -> Self {
        let conn = midir::MidiOutput::new("Harmonia").expect("creating midi output connection");
        let ports = conn.ports();

        #[cfg(unix)]
        let virtual_port = {
            use midir::os::unix::VirtualOutput;
            Arc::new(Mutex::new(
                midir::MidiOutput::new("HarmoniaVirt")
                    .expect("creating midi output connection")
                    .create_virtual("Harmonia")
                    .expect("creating virtual midi port: {}"),
            ))
        };

        Self {
            conn: Arc::new(Mutex::new(conn)),
            ports,
            #[cfg(unix)]
            virtual_port,
        }
    }
}

impl MidiConnection {
    /// Update list of currently known [MidiOutputPort]s
    pub fn refresh(&mut self) {
        self.ports = self.conn.lock().unwrap().ports();
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

    /// Inform server that user requested application to stop
    ///
    /// Used to stop application from the HTTP handlers
    pub abort: tokio::sync::Notify,

    /// Nick that helps users to identify each others
    pub nick: tokio::sync::RwLock<String>,
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
    fn new(cli: &Cli) -> Self {
        let link = Arc::new(AblLink::new(120.));
        link.enable(!cli.disable_link);

        let nick = std::fs::read_to_string(cache_path().join(NICK_PATH)).unwrap_or_else(|_| {
            let username = whoami::realname();
            tracing::warn!("Failed to find a nick file, using username {username:?}");
            username
        });

        Self {
            blocks: Default::default(),
            connection: Default::default(),
            link: link.clone(),
            audio_engine: Default::default(),
            currently_playing_uuid: Default::default(),
            current_playing_progress: Default::default(),
            port: cli.port,
            groups: Some(linky_groups::listen(link)),
            abort: Default::default(),
            nick: tokio::sync::RwLock::new(nick),
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
struct Cli {
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

    /// Disable colors. Overwrites NO_COLOR environment variable
    #[arg(long = "no-color", default_value_t = false)]
    disable_colors: bool,
}

/// Initialize Harmonia logging system
///
/// Harmonia logs all the events inside log files, each file timestamped by day.
fn setup_logging_system(cli: &Cli) -> tracing_appender::non_blocking::WorkerGuard {
    let log_file_appender = tracing_appender::rolling::daily(log_path(), "logs");
    let (log_file_appender, guard) = tracing_appender::non_blocking(log_file_appender);

    // https://no-color.org/
    let disable_colors = cli.disable_colors
        || std::env::var("NO_COLOR")
            .map(|x| !x.is_empty())
            .unwrap_or(false);

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "harmonia=info,linky_groups=info,linky_groups::net=info".into()
            }),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(!disable_colors)
                .and_then(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(log_file_appender),
                ),
        )
        .init();
    guard
}

/// Setup Linux specific env for application
#[cfg(target_os = "linux")]
fn os_specific_initialization() {}

/// Setup macOS specific env for application
#[cfg(target_os = "macos")]
fn os_specific_initialization() {}

/// Setup Windows specific env for application
///
/// Enables processing ANSI escape codes to properly display colors in cmd.exe
#[cfg(target_os = "windows")]
fn os_specific_initialization() {
    use winapi::{
        shared::minwindef::{DWORD, TRUE},
        um::consoleapi::{GetConsoleMode, SetConsoleMode},
        um::handleapi::INVALID_HANDLE_VALUE,
        um::processenv::GetStdHandle,
        um::winbase::STD_OUTPUT_HANDLE,
        um::wincon::ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    };
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle == INVALID_HANDLE_VALUE {
            return;
        }
        let mut mode: DWORD = 0;
        if GetConsoleMode(handle, &mut mode as *mut DWORD) != TRUE {
            return;
        }
        mode |= ENABLE_VIRTUAL_TERMINAL_PROCESSING;
        SetConsoleMode(handle, mode);
    }
}

/// Initialize Harmonia application
///
/// Setup all synchronization mechanisms, HTTP server, recollect stored [AppState] from [cache],
/// enable [logging mechanism][logs], mount [handlers] to HTTP serer, listen at provided HTTP port
/// and ensure proper cleanup on exit.
///
/// [cache]: cache_path()
/// [logs]: setup_logging_system()
#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    os_specific_initialization();

    let cli = Cli::parse();
    let _guard = setup_logging_system(&cli);

    tracing::info!("starting up version {}", Version::default());

    let app_state = Arc::new(AppState::new(&cli));
    if let Err(err) = app_state.recollect_previous_blocks() {
        tracing::warn!("trying to recollect previous sources: {err:#}")
    } else {
        tracing::debug!(
            "recollected {count} blocks",
            count = app_state.blocks.read().unwrap().len()
        )
    }

    app_state.audio_engine.write().unwrap().state = Arc::downgrade(&app_state);
    tracing::debug!(
        "link {}",
        if cli.disable_link {
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
            any(link_status_websocket_handler),
        )
        .route("/blocks/midi", put(handlers::add_new_midi_source_block))
        .route(
            "/blocks/shared_memory",
            put(handlers::add_new_shered_memory_block),
        )
        .route("/blocks/{uuid}", delete(handlers::remove_block))
        .route("/blocks/{uuid}", get(handlers::download_block_content))
        .route("/blocks/play/{uuid}", post(handlers::play))
        .route(
            "/blocks/midi/set-port/{uuid}",
            post(handlers::set_port_for_midi),
        )
        .route("/nick", post(handlers::set_nick))
        .route("/nick", get(handlers::nick))
        .route("/blocks/set-group/{uuid}", post(handlers::set_group))
        .route("/blocks/set-keybind/{uuid}", post(handlers::set_keybind))
        .route("/interrupt", post(handlers::interrupt))
        .route("/abort", post(handlers::abort))
        .route("/", get(handlers::index))
        .route("/htmx.min.js", public::static_response!(get, "htmx.min.js"))
        .route("/index.js", public::static_response!(get, "index.js"))
        .route("/index.css", public::static_response!(get, "index.css"))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .with_state(app_state.clone());

    let ip: IpAddr = cli.ip.parse().unwrap();
    let addr = SocketAddr::from((ip, cli.port));
    let Ok(listener) = tokio::net::TcpListener::bind(addr).await else {
        tracing::error!("Address already in use at http://{addr}");
        return ExitCode::FAILURE;
    };

    let display_address = if addr.ip().is_unspecified() {
        SocketAddr::new(Ipv4Addr::new(127, 0, 0, 1).into(), addr.port())
    } else {
        addr
    };

    let server = {
        let app_state = app_state.clone();

        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
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

            let user_requested_abort = app_state.abort.notified();

            tokio::select! {
                _ = ctrl_c => {},
                _ = terminate => {},
                _ = user_requested_abort => {},
            }
        })
    };

    tracing::info!("Connect to Harmonia, which is available at http://{display_address}");
    if cli.open {
        tracing::debug!("opening UI in default browser");
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
    tracing::info!("websocket connect: addr={addr}, user_agent={user_agent}");
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
    // TODO: Sleep should be based on BPM to keep in sync with clock as good as possible
    let mut interval = tokio::time::interval(Duration::from_millis(100));

    loop {
        interval.tick().await;
        let markup = html! {
            (handlers::runtime_status(app_state.clone()).await);
            (handlers::playing_status(app_state.clone()).await);
        };

        if let Err(err) = socket
            .send(Message::Text(markup.into_string().into()))
            .await
        {
            tracing::error!("websocket send to {addr} failed: {err}");
            break;
        }
    }
    if let Err(e) = socket
        .send(Message::Close(Some(axum::extract::ws::CloseFrame {
            code: axum::extract::ws::close_code::ERROR,
            reason: Utf8Bytes::from_static("Failed to communicate with the server"),
        })))
        .await
    {
        tracing::error!("websocket close to {addr} failed: {e}");
    }
}
