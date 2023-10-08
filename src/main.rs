use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

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
    routing::{get, post},
    Router, TypedHeader,
};

use midir::{MidiOutput, MidiOutputPort};
use midly::SmfBytemap;
use rusty_link::{AblLink, SessionState};
use tower_http::{
    services::ServeDir,
    trace::{DefaultMakeSpan, TraceLayer},
};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use maud::{html, Markup, DOCTYPE};

use sha1::{Digest, Sha1};

mod audio_engine;
use audio_engine::AudioEngine;

pub struct MidiSource {
    pub bytes: Vec<u8>,
    pub file_name: String,
    pub associated_port: usize,
}

impl MidiSource {
    pub fn midi(&self) -> Result<SmfBytemap<'_>, midly::Error> {
        SmfBytemap::parse(&self.bytes)
    }
}

type MidiSources = HashMap<String, MidiSource>;

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
    pub sources: RwLock<MidiSources>,
    pub connection: RwLock<MidiConnection>,
    pub link: AblLink,
    pub audio_engine: RwLock<AudioEngine>,
}

impl AppState {
    fn new() -> Self {
        Self {
            sources: Default::default(),
            connection: Default::default(),
            link: AblLink::new(120.),
            audio_engine: Default::default(),
        }
    }
}

fn version() -> String {
    format!(
        "{pkg_version} ({hash} {date}{dirty})",
        pkg_version = env!("CARGO_PKG_VERSION"),
        hash = env!("GIT_STATUS_HASH"),
        date = build_time::build_time_local!("%Y-%m-%d %H:%M"),
        dirty = {
            let dirty = env!("GIT_STATUS_DIRTY");
            if dirty == "dirty" {
                " dirty"
            } else {
                ""
            }
        }
    )
}

#[tokio::main]
async fn main() {
    let do_help = std::env::args().any(|param| &param == "--help" || &param == "-h");
    let do_open = std::env::args().any(|param| &param == "--open");
    let disable_link = std::env::args().any(|param| &param == "--disable-link");

    if do_help {
        help_and_exit();
    }

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "harmonia=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("starting up version {}", version());

    let app_state = Arc::new(AppState::new());
    app_state.audio_engine.write().unwrap().state = Arc::downgrade(&app_state);
    app_state.link.enable(!disable_link);
    info!(
        "link {}",
        if disable_link { "not active" } else { "active" }
    );

    let public_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("public");

    let app = Router::new()
        .fallback_service(ServeDir::new(public_dir))
        .route("/api/health", get(health_handler))
        .route(
            "/api/link-status-websocket",
            get(link_status_websocket_handler),
        )
        .route("/link/status", get(link_status_handler))
        .route("/midi/add", post(midi_add_handler))
        .route("/midi/play/:uuid", post(midi_play_handler))
        .route("/midi/ports", get(midi_ports_handler))
        .route("/midi/download/:uuid", get(midi_download_handler))
        .route("/version", get(version_handler))
        .route("/", get(index_handler))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .with_state(app_state.clone());

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("listening on http://{addr}");
    let server =
        axum::Server::bind(&addr).serve(app.into_make_service_with_connect_info::<SocketAddr>());

    if do_open {
        info!("opening UI in default browser");
        open::that_detached(format!("http://{addr}")).unwrap();
    }

    server.await.unwrap();
    app_state.link.enable(false);
}

fn help_and_exit() -> ! {
    println!("harmonia [--open] [--help]");
    println!("  --open - opens UI in default browser");
    println!("  --help - prints this message");
    std::process::exit(0);
}

async fn local_ips_handler() -> Markup {
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

    html! {
        details {
            summary {
                "Local IP addresses"
            }

            ul {
                @for (iface, ip) in interfaces {
                    @if !ip.is_loopback() {
                        li {
                            (format!("{iface} - {ip}"));
                        }
                    }
                }
            }
        }
    }
}

async fn link_status_handler(State(app_state): State<Arc<AppState>>) -> Markup {
    let mut session_state = SessionState::default();
    app_state.link.capture_app_session_state(&mut session_state);
    let time = app_state.link.clock_micros();

    // TODO: Move quantum to state
    let quantum = 4.0;

    let beat = session_state.beat_at_time(time, quantum);

    html! {
        "BPM: ";    (session_state.tempo());
        ", beat: "; (beat);
        ", playing: "; (session_state.is_playing());
    }
}

async fn midi_download_handler(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
) -> Response<Full<Bytes>> {
    let midi_sources = app_state.sources.read().unwrap();
    let Some(midi_source) = midi_sources.get(&uuid) else {
        error!("{uuid} not found");
        let mut response = Response::new(Full::from("not found"));
        *response.status_mut() = StatusCode::NOT_FOUND;
        response.headers_mut().insert(CONTENT_TYPE, "text/html".parse().unwrap());
        return response;
    };

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

async fn midi_sources_render(app_state: State<Arc<AppState>>) -> Markup {
    let midi_sources = app_state.sources.read().unwrap();

    html! {
        table {
            thead {
                th { "Filename" }
                th { "Info" }
                th { "Associated port" }
                th { "Keybind" }
                th { "Controls" }
            }
            tbody {
                @for (uuid, source) in midi_sources.iter() {
                    tr data-uuid=(uuid) {
                        td {
                            a href=(format!("/midi/download/{uuid}")) {
                                (source.file_name)
                            }
                        }
                        @match source.midi() {
                            Ok(midi) => td {
                                "Format: ";
                                ({
                                    match midi.header.format {
                                        midly::Format::Sequential | midly::Format::SingleTrack => "sequential",
                                        midly::Format::Parallel => "parallel",
                                    }
                                });
                                ", tracks count: ";
                                (midi.tracks.len());
                            },
                            Err(err) => td {
                                "Failed to parse MIDI file: ";
                                (err);
                            },
                        }
                        td {
                            input
                                type="number" value=(source.associated_port)
                                hx-post=(format!("/midi/set-port/{uuid}/"));
                        }
                        td {
                            input
                                type="text";
                        }
                        td hx-target="this" {
                            (render_play_cell(uuid, false, None))
                        }
                    }
                }
            }
        }
    }
}

fn render_play_cell(uuid: &str, playing: bool, error_message: Option<String>) -> Markup {
    html! {
        button hx-post=(format!("/midi/play/{uuid}")) hx-swap="innerHTML" {
            // https://en.wikipedia.org/wiki/Media_control_symbols
            (if !playing || error_message.is_some() { "▶" } else { "⏹" })
        }
        @if let Some(error_message) = error_message {
            div style="color: red" {
                (error_message)
            }
        }
    }
}

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
                }
                main {
                    h2 { "Link status" }
                    div id="link-status" {
                        (link_status_handler(app_state.clone()).await)
                    }
                    h2 { "System information" }
                    (local_ips_handler().await);
                    h3 { "MIDI ports" }
                    button hx-get="/midi/ports" hx-target="#midi-ports" hx-swap="innerHTML" {
                        "Refresh"
                    }
                    div id="midi-ports" {
                        (midi_ports_handler(app_state.clone()).await);
                    }
                    h2 { "MIDI sources" }
                    form
                        hx-post="/midi/add"
                        hx-target="#midi-sources-list"
                        hx-swap="innerHTML"
                        hx-encoding="multipart/form-data"
                    {
                        input id="midi-sources-input" name="files" type="file" multiple accept="audio/midi";
                        button { "Upload" }
                    }
                    div id="midi-sources-list" {
                        (midi_sources_render(app_state).await);
                    }
                }
            }
        }
    }
}

async fn health_handler() -> &'static str {
    "Hi"
}

async fn version_handler() -> Markup {
    html! {
        (format!("Version: {}", version()));
    }
}

async fn midi_ports_handler(State(app_state): State<Arc<AppState>>) -> Markup {
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

async fn midi_play_handler(
    State(app_state): State<Arc<AppState>>,
    Path(uuid): Path<String>,
) -> Markup {
    let started_playing = audio_engine::play(app_state.clone(), &uuid).await;
    render_play_cell(
        &uuid,
        true,
        if let Err(error_message) = started_playing {
            error!("failed to play requested {uuid}: {error_message}");
            Some(error_message)
        } else {
            None
        },
    )
}

async fn midi_add_handler(
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

        let midi_source = MidiSource {
            bytes: data,
            file_name: file_name.clone(),
            associated_port: 1,
        };

        let midi_sources = &mut app_state.sources.write().unwrap();
        midi_sources.insert(uuid, midi_source);
    }

    midi_sources_render(axum::extract::State(app_state)).await
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
