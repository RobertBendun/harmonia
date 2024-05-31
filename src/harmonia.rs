use clap::Parser;
use std::{
    collections::HashMap,
    io::BufReader,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    process::ExitCode,
    sync::{Arc, RwLock},
    time::Duration,
};

use anyhow::Context;
use axum::{
    body::{Bytes, Full},
    extract::{
        ws::{Message, WebSocket},
        ConnectInfo, Multipart, Path, State, WebSocketUpgrade,
    },
    http::{
        header::{CONTENT_DISPOSITION, CONTENT_TYPE},
        Response, StatusCode,
    },
    response::IntoResponse,
    routing::{delete, get, post, put},
    Form, Router, TypedHeader,
};

use midir::{MidiOutput, MidiOutputPort};
use rusty_link::{AblLink, SessionState};
use serde::Deserialize;
use tower_http::trace::{DefaultMakeSpan, TraceLayer};
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

use maud::{html, Markup, DOCTYPE};

use sha1::{Digest, Sha1};

mod audio_engine;
use audio_engine::AudioEngine;
mod version;
use version::Version;
mod block;
mod public;

const STATE_PATH: &str = "harmonia_state.bson";

pub struct MidiConnection {
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
    pub fn refresh(&mut self) {
        // TODO: Is it valid to create a new MidiOutput per use? Maybe we should create only one
        // MidiOutput port per application.
        let out = MidiOutput::new("harmonia").unwrap();
        self.ports = out.ports();
    }
}

pub struct AppState {
    pub blocks: RwLock<HashMap<String, block::Block>>,
    pub connection: RwLock<MidiConnection>,
    pub link: Arc<AblLink>,
    pub audio_engine: RwLock<AudioEngine>,
    // TODO: Be better
    pub currently_playing_uuid: RwLock<Option<String>>,
    pub current_playing_progress: RwLock<(usize, usize)>,
    pub port: u16,
    pub groups: Option<linky_groups::Groups>,
}

fn cache_path() -> PathBuf {
    let path = dirs::cache_dir()
        .expect("documentation states that this function should work on all platforms")
        .join("harmonia");
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn log_path() -> PathBuf {
    cache_path()
}

impl AppState {
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

    fn recollect_previous_blocks(&self) -> Result<(), anyhow::Error> {
        let path = cache_path().join(STATE_PATH);
        let file = std::fs::File::open(path).context("opening state file")?;

        let new_sources: HashMap<String, block::Block> =
            bson::from_reader(BufReader::new(file)).context("reading bson file")?;
        let mut sources = self.blocks.write().unwrap();
        sources.extend(new_sources);

        Ok(())
    }

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

    async fn htmx_js() -> impl IntoResponse {
        public::File("htmx.min.js")
    }
    async fn index_js() -> impl IntoResponse {
        public::File("index.js")
    }

    // Conventions:
    //   Paths begining with /api/ are meant for JavaScript
    //   Others are for HTML / HTMLX consumption
    let app = Router::new()
        .route(
            "/api/link-status-websocket",
            get(link_status_websocket_handler),
        )
        .route("/api/link-switch-enabled", post(link_switch_enabled))
        .route("/link/status", get(link_status_handler))
        .route("/blocks/midi", put(add_new_midi_source_block))
        .route("/blocks/shared_memory", put(add_new_shered_memory_block))
        .route("/blocks/:uuid", delete(remove_block))
        .route("/blocks/:uuid", get(block_download_content))
        .route("/blocks/play/:uuid", post(play_block))
        .route("/midi/ports", get(midi_list_ports_handler))
        .route("/blocks/midi/set-port/:uuid", post(block_set_port_for_midi))
        .route("/blocks/set-group/:uuid", post(block_set_group))
        .route("/blocks/set-keybind/:uuid", post(block_set_keybind))
        .route("/version", get(version_handler))
        .route("/interrupt", post(interrupt))
        .route("/", get(index_handler))
        .route("/htmx.min.js", get(htmx_js))
        .route("/index.js", get(index_js))
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

async fn system_information(app_state: State<Arc<AppState>>) -> Markup {
    let port = app_state.port;
    let mut interfaces = match local_ip_address::list_afinet_netifas() {
        Ok(list) => list,
        Err(err) => {
            error!("failed to retrive local ips: {err}");
            return html! {
                p {
                    "failed to retrive local ips"
                }
            };
        }
    };

    interfaces.sort_by(|(if1, _), (if2, _)| if1.cmp(if2));

    let hostname = gethostname::gethostname();
    let hostname = hostname.to_string_lossy();

    html! {
        details {
            summary {
                "Hostname, IP address"
            }

            p {
                "Hostname: "; a href=(format!("http://{hostname}:{port}")) {
                    (hostname)
                }
            }

            @if let Ok(local_ip) = local_ip_address::local_ip() {
                p {
                    "Local IP: ";
                    (local_ip)
                }
            } @else {
                ul {
                    @for (iface, ip) in interfaces {
                        @if !ip.is_loopback() {
                            li {
                                (format!("{iface} -"));
                                a href=(format!("http://{ip}:{port}")) {
                                    (ip);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn link_status_handler(State(app_state): State<Arc<AppState>>) -> Markup {
    let mut session_state = SessionState::default();
    let active = app_state.link.is_enabled();
    app_state.link.capture_app_session_state(&mut session_state);
    let time = app_state.link.clock_micros();

    // TODO: Move quantum to state
    let quantum = 1.0;

    let beat = session_state.beat_at_time(time, quantum);

    let currently_playing = app_state.currently_playing_uuid.read().unwrap();

    let current_playing_progress = *app_state.current_playing_progress.read().unwrap();

    let peers = app_state.link.num_peers();

    html! {
        div {
            "Active: "; (active);
            ", Peers: "; (peers);
            ", BPM: ";    (session_state.tempo());
            ", beat: "; (beat);
            ", playing: "; (app_state.groups.as_ref().unwrap().is_playing());
        }
        @if let Some(currently_playing) = &*currently_playing {
            div {
                "Currently playing: ";
                ({
                    app_state.blocks.read().unwrap().get(currently_playing).unwrap().content.name()
                });
                " ";
                progress max=(current_playing_progress.1) min="0" value=(current_playing_progress.0) {}
            }
        }
    }
}

// TODO:  This triplets of {SetX, midi_set_x_for_source, render_x_cell} maybe should be
// consolidated

#[derive(Deserialize)]
struct SetPort {
    pub port: usize,
}

async fn block_set_port_for_midi(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
    Form(SetPort { port }): Form<SetPort>,
) -> Result<Markup, StatusCode> {
    let mut blocks = app_state.blocks.write().unwrap();

    let Some(block) = blocks.get_mut(&uuid) else {
        error!("block#{uuid} was not found");
        return Err(StatusCode::NOT_FOUND);
    };

    let block::Content::Midi(ref mut midi) = block.content else {
        error!("block#{uuid} is not a MIDI source");
        return Err(StatusCode::BAD_REQUEST);
    };

    let min = 1_usize;
    let max = app_state.connection.read().unwrap().ports.len();
    if port < min || port > max {
        error!("port number should be between {min} and {max}");
        return Ok(render_port_cell(
            &uuid,
            midi.associated_port,
            Some(if port > max {
                "port number too high".to_string()
            } else {
                "port number too low".to_string()
            }),
        ));
    }

    info!("setting port {port} for {uuid}");
    midi.associated_port = port - 1;
    Ok(render_port_cell(&uuid, midi.associated_port, None))
}

#[derive(Deserialize)]
struct SetGroup {
    pub group: String,
}

async fn block_set_group(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
    Form(SetGroup { group }): Form<SetGroup>,
) -> Result<Markup, StatusCode> {
    let response = {
        let mut blocks = app_state.blocks.write().unwrap();

        let Some(midi_source) = blocks.get_mut(&uuid) else {
            error!("block#{uuid} not found");
            return Err(StatusCode::NOT_FOUND);
        };

        // TODO: Unnesesary string allocation
        midi_source.group = if group.len() > linky_groups::MAX_GROUP_ID_LENGTH {
            let mut cut = linky_groups::MAX_GROUP_ID_LENGTH;
            while !group.is_char_boundary(cut) {
                cut -= 1;
            }
            &group[..cut]
        } else {
            &group[..]
        }
        .to_owned();

        tracing::info!(
            "Switched block#{uuid} to group {group:?}",
            group = midi_source.group
        );
        Ok(render_group_cell(&uuid, &midi_source.group))
    };

    if let Err(err) = app_state.remember_current_blocks() {
        error!("block_set_group failed to remember current sources: {err:#}")
    }
    response
}

#[derive(Deserialize)]
struct SetKeybind {
    pub keybind: String,
}

async fn block_set_keybind(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
    Form(SetKeybind { keybind }): Form<SetKeybind>,
) -> StatusCode {
    {
        let mut midi_sources = app_state.blocks.write().unwrap();

        let Some(block) = midi_sources.get_mut(&uuid) else {
            error!("block#{uuid} not found");
            return StatusCode::NOT_FOUND;
        };

        info!("Changing keybind for block#{uuid} to {keybind}");
        block.keybind = keybind;
    }

    if let Err(err) = app_state.remember_current_blocks() {
        error!("set_keybind failed to remember current sources: {err:#}")
    }

    StatusCode::OK
}

async fn block_download_content(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
) -> Response<Full<Bytes>> {
    let not_found = || {
        let mut response = Response::new(Full::from("not found"));
        *response.status_mut() = StatusCode::NOT_FOUND;
        response
            .headers_mut()
            .insert(CONTENT_TYPE, "text/html".parse().unwrap());
        response
    };

    let blocks = app_state.blocks.read().unwrap();
    let Some(block) = blocks.get(&uuid) else {
        error!("block#{uuid} not found");
        return not_found();
    };

    match &block.content {
        block::Content::SharedMemory { .. } => not_found(),
        block::Content::Midi(midi_source) => {
            // TODO: Unnesesary clone?
            let mut response = Response::new(Full::from(midi_source.bytes.clone()));
            let headers = &mut response.headers_mut();
            headers.insert(
                CONTENT_DISPOSITION,
                format!("attachement; filename=\"{}\"", midi_source.file_name)
                    .parse()
                    .unwrap(),
            );
            headers.insert(CONTENT_TYPE, "audio/midi".parse().unwrap());
            response
        }
    }
}

async fn remove_block(app_state: State<Arc<AppState>>, Path(uuid): Path<String>) -> Markup {
    {
        let mut sources = app_state.blocks.write().unwrap();
        sources.remove(&uuid);
    }
    if let Err(err) = app_state.remember_current_blocks() {
        error!("remove_midi_source_handler failed to remember current sources: {err:#}")
    }

    render_blocks(app_state).await
}

async fn render_blocks(app_state: State<Arc<AppState>>) -> Markup {
    use block::Content;

    let blocks = app_state.blocks.read().unwrap();
    let mut orderered_blocks: Vec<_> = blocks.iter().collect();
    orderered_blocks.sort_by(|(_, lhs), (_, rhs)| match (lhs.order, rhs.order) {
        (Some(lhs), Some(rhs)) => lhs.cmp(&rhs),
        (Some(_), _) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => match (&lhs.content, &rhs.content) {
            (Content::Midi(lhs), Content::Midi(rhs)) => lhs.file_name.cmp(&rhs.file_name),
            (Content::SharedMemory { path: lhs }, Content::SharedMemory { path: rhs }) => {
                lhs.cmp(rhs)
            }
            (Content::SharedMemory { .. }, Content::Midi(_)) => std::cmp::Ordering::Less,
            (Content::Midi(_), Content::SharedMemory { .. }) => std::cmp::Ordering::Greater,
        },
    });

    html! {
        @for (uuid, block) in orderered_blocks.iter() {
            section {
                @match &block.content {
                    Content::Midi(source) => {
                        a href=(format!("/blocks/{uuid}")) { (source.file_name) }
                        (render_port_cell(uuid, source.associated_port, None));
                    }
                    Content::SharedMemory { path } => {
                        input
                            name="path"
                            data-uuid=(uuid)
                            type="text"
                            hx-post=("TODO !!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!")
                            hx-swap="none"
                            value=(path);
                    }
                }

                div { (render_group_cell(uuid, &block.group)) }

                input
                    name="keybind"
                    data-uuid=(uuid)
                    onchange="update_key_binding(this)"
                    type="text"
                    hx-post=(format!("/blocks/set-keybind/{uuid}"))
                    hx-swap="none"
                    value=(block.keybind);

                (render_controls_cell(uuid, None))
            }
        }
    }
}

fn render_controls_cell(uuid: &str, error_message: Option<String>) -> Markup {
    html! {
        div {
            button hx-target="closest div" hx-post=(format!("/blocks/play/{uuid}")) {
                // https://en.wikipedia.org/wiki/Media_control_symbols
                "▶"
            }
            button
                hx-delete=(format!("/blocks/{uuid}"))
                hx-target="#blocks-list"
                hx-swap="innerHTML"
                {
                    "delete"
                }
            @if let Some(error_message) = error_message {
                div style="color: red" {
                    (error_message)
                }
            }
        }
    }
}

fn render_port_cell(uuid: &str, associated_port: usize, error_message: Option<String>) -> Markup {
    html! {
        input
            type="number" value=(format!("{}", associated_port + 1))
            name="port"
            hx-target="closest div"
            hx-post=(format!("/blocks/set-port/{uuid}"));

        @if let Some(error_message) = error_message {
            div style="color: red" {
                (error_message)
            }

        }
    }
}

fn render_group_cell(uuid: &str, group: &str) -> Markup {
    html! {
        input
            type="text" value=(group)
            pattern=(format!("(\\w| ){{0,{}}}", linky_groups::MAX_GROUP_ID_LENGTH))
            maxlength=(linky_groups::MAX_GROUP_ID_LENGTH)
            name="group"
            hx-target="closest div"
            hx-post=(format!("/blocks/set-group/{uuid}"));
    }
}

// TODO: Shorten state cache path when possible - like /home/user/foo to ~/foo
async fn index_handler(app_state: State<Arc<AppState>>) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            (DOCTYPE)
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Harmonia control panel" }
                script src="index.js" {}
                script src="htmx.min.js" {}
            }
            body {
                noscript {
                    div style="font-weight: bold; color: red; font-size: 1.1em; max-width: 800px; margin: 1em auto 1em auto" {
                        "This is a web application, and thus requires JavaScript to work. But fear not!
                         This code is a free software (AGPL 3+) with at least open source dependencies:";
                        ul {
                            li { a href="https://github.com/RobertBendun/harmonia" { "harmonia" }  " - app that you are seeing now"; }
                            li { a href="https://htmx.org/" { "htmx" } }
                        }
                    }
                }
                header {
                    h1 { "Harmonia control panel" }
                    "Status: "
                    span id="app-health" {}
                    br;
                    (version_handler().await);
                    br;
                    "Logs and state: ";
                    ({
                        let cache = cache_path();
                        let c: &std::path::Path = &cache;
                        dirs::home_dir()
                            .and_then(|home| c.strip_prefix(home).ok())
                            .map(|p| PathBuf::from("~").join(p))
                            .unwrap_or(cache)
                            .to_str()
                            .unwrap()
                            .to_owned()
                    });
                    br;
                    button onclick="toggle_color_scheme()" {
                        "Toggle color scheme"
                    }
                }
                main {
                    h2 { "Runtime status" }
                    button onclick="change_link_status()" {
                        "Change link status"
                    }
                    button hx-post="/interrupt" hx-swap="none" {
                        "Interrupt MIDI"
                    }
                    div id="link-status" {
                        (link_status_handler(app_state.clone()).await)
                    }
                    h2 { "System information" }
                    (system_information(app_state.clone()).await);
                    h3 { "MIDI ports" }
                    p {
                        "Refresh page (F5 or CTRL-R) to refresh list of available MIDI ports";
                    }
                    div id="midi-ports" {
                        (midi_list_ports_handler(app_state.clone()).await);
                    }
                    h2 { "Blocks" }
                    form
                        hx-put="/blocks/midi"
                        hx-target="#blocks-list"
                        hx-swap="innerHTML"
                        hx-encoding="multipart/form-data"
                    {
                        input id="midi-sources-input" name="files" type="file" multiple accept="audio/midi";
                        button { "Upload" }
                    }
                    form
                        hx-put="/blocks/shared_memory"
                        hx-target="#blocks-list"
                        hx-swap="innerHTML"
                    {
                        input id="path" name="path" type="text";
                        button { "Add" }
                    }

                    div id="blocks-list" {
                        (render_blocks(app_state).await);
                    }
                }
            }
        }
    }
}

async fn version_handler() -> Markup {
    html! {
        "Version: ";
        (Version::default());
    }
}

async fn midi_list_ports_handler(State(app_state): State<Arc<AppState>>) -> Markup {
    let out = MidiOutput::new("harmonia").unwrap();

    let mut midi_conn = app_state.connection.write().unwrap();
    midi_conn.refresh();

    let ports = midi_conn
        .ports
        .iter()
        .filter_map(|port| Result::ok(out.port_name(port)));

    html! {
        ol {
            @for port_name in ports {
                li { (port_name) }
            }
        }
    }
}

#[axum::debug_handler]
async fn play_block(State(app_state): State<Arc<AppState>>, Path(uuid): Path<String>) -> Markup {
    let started_playing = audio_engine::play(app_state.clone(), &uuid).await;
    render_controls_cell(
        &uuid,
        if let Err(error_message) = started_playing {
            error!("failed to play requested {uuid}: {error_message}");
            Some(error_message)
        } else {
            None
        },
    )
}

async fn interrupt(State(app_state): State<Arc<AppState>>) {
    if let Err(error) = audio_engine::interrupt(app_state).await {
        tracing::error!("failed to interrupt: {error}");
    }
}

#[derive(Deserialize)]
struct AddSharedMemoryBlock {
    path: String,
}

async fn add_new_shered_memory_block(
    State(app_state): State<Arc<AppState>>,
    Form(AddSharedMemoryBlock { path }): Form<AddSharedMemoryBlock>,
) -> Markup {
    let mut hasher = Sha1::new();
    hasher.update(path.as_bytes());
    let uuid = hex::encode(hasher.finalize());

    let content = block::Content::SharedMemory { path };

    let block = block::Block {
        content,
        group: Default::default(),
        keybind: Default::default(),
        order: Default::default(),
    };

    {
        let blocks = &mut app_state.blocks.write().unwrap();
        blocks.insert(format!("shm-{uuid}"), block);
    }

    if let Err(err) = app_state.remember_current_blocks() {
        error!("add_new_shered_memory_block failed to remember current sources: {err:#}")
    }

    render_blocks(axum::extract::State(app_state)).await
}

async fn add_new_midi_source_block(
    State(app_state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Markup {
    while let Some(field) = multipart.next_field().await.unwrap() {
        // TODO: Check that this is the name that we are expecting
        let _name = field.name().unwrap().to_string();
        // TODO: Better default file name
        let file_name = field.file_name().unwrap_or("<unknown>").to_string();
        let data = field.bytes().await.unwrap().to_vec();
        let mut hasher = Sha1::new();
        hasher.update(&data);
        let uuid = hex::encode(hasher.finalize());

        let midi_source = block::MidiSource {
            bytes: data,
            file_name: file_name.clone(),
            associated_port: 0,
        };

        let block = block::Block {
            content: block::Content::Midi(midi_source),
            group: Default::default(),
            keybind: Default::default(),
            order: Default::default(),
        };

        let midi_sources = &mut app_state.blocks.write().unwrap();
        midi_sources.insert(format!("midi-{uuid}"), block);
    }

    if let Err(err) = app_state.remember_current_blocks() {
        error!("add_new_midi_source_block failed to remember current sources: {err:#}")
    }

    render_blocks(axum::extract::State(app_state)).await
}

async fn link_switch_enabled(State(app_state): State<Arc<AppState>>) {
    app_state.link.enable(!app_state.link.is_enabled());
}

// For expanding this websocket buisness see: https://github.com/tokio-rs/axum/blob/main/examples/websockets/src/main.rs
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

async fn link_status_websocket_loop(
    mut socket: WebSocket,
    addr: SocketAddr,
    app_state: State<Arc<AppState>>,
) {
    loop {
        let markup = link_status_handler(app_state.clone()).await;
        if let Err(err) = socket.send(Message::Text(markup.into_string())).await {
            error!("websocket send to {addr} failed: {err}");
            break;
        }
        // TODO: Sleep should be based on BPM to keep in sync with clock as good as possible
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = socket.close().await;
}
