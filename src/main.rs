use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        ConnectInfo, Multipart, Path, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router, TypedHeader,
};
use midir::MidiOutput;
use midly::SmfBytemap;
use tower_http::{
    services::ServeDir,
    trace::{DefaultMakeSpan, TraceLayer},
};
use tracing::{error, info, instrument};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

struct MidiSource {
    bytes: Vec<u8>,
    file_name: String,
}

impl MidiSource {
    fn midi<'a>(&'a self) -> Result<SmfBytemap<'a>, midly::Error> {
        SmfBytemap::parse(&self.bytes)
    }

    #[instrument]
    fn play() {}
}

type MidiSources = HashMap<String, MidiSource>;

#[tokio::main]
async fn main() {
    let do_help = std::env::args()
        .find(|param| param == "--help" || param == "-h")
        .is_some();
    let do_open = std::env::args().find(|param| param == "--open").is_some();

    if do_help {
        help_and_exit();
    }

    let midi_sources = Arc::new(RwLock::new(MidiSources::default()));

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "harmonia=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("starting up commit {}", env!("GIT_INFO"));

    let public_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("public");

    let app = Router::new()
        .fallback_service(ServeDir::new(public_dir).append_index_html_on_directories(true))
        .route("/api/health", get(health_handler))
        .route("/api/ws", get(websocket_handler))
        .route("/api/midi/add", post(midi_add_handler))
        .route("/api/midi/play/:uuid", post(midi_play_handler))
        .route("/api/midi/ports", get(midi_ports_handler))
        .route("/api/version", get(version_handler))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .layer(Extension(midi_sources));

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("listening on http://{addr}");
    let server =
        axum::Server::bind(&addr).serve(app.into_make_service_with_connect_info::<SocketAddr>());

    if do_open {
        open::that_detached(format!("http://{addr}")).unwrap();
    }

    server.await.unwrap();
}

fn help_and_exit() -> ! {
    println!("harmonia [--open] [--help]");
    println!("  --open - opens UI in default browser");
    println!("  --help - prints this message");
    std::process::exit(0);
}

async fn health_handler() -> &'static str {
    "Hi"
}

async fn version_handler() -> impl IntoResponse {
    #[derive(serde::Serialize)]
    struct VersionInfo {
        version: &'static str,
        commit: &'static str,
        dirty: bool,
    }
    let info: Vec<&str> = env!("GIT_INFO").split('_').collect();
    let (commit, dirty) = if info.len() == 2 {
        (info[1], true)
    } else {
        (info[0], false)
    };
    Json(VersionInfo {
        version: env!("CARGO_PKG_VERSION"),
        commit,
        dirty,
    })
}

async fn midi_ports_handler() -> Json<Vec<String>> {
    let out = MidiOutput::new("harmonia").unwrap();

    Json(
        out.ports()
            .iter()
            .filter_map(|port| Result::ok(out.port_name(port)))
            .collect(),
    )
}

// use axum::debug_handler;
// #[debug_handler]
async fn midi_play_handler(
    midi_sources: Extension<Arc<RwLock<MidiSources>>>,
    Path(uuid): Path<String>,
) -> Json<()> {
    let midi_sources = midi_sources.read().unwrap();
    let Some(midi_source) = midi_sources.get(&uuid) else {
        println!("not found");
        return Json(());
    };
    let midi = midi_source.midi().unwrap();

    let midi_out = MidiOutput::new("harmonia").unwrap();
    let midi_port = &midi_out.ports()[0];
    info!(
        "connected to output port: {}",
        midi_out.port_name(midi_port).unwrap()
    );
    let mut conn_out = midi_out
        .connect(midi_port, /* TODO: Better name */ "play")
        .unwrap();

    for (bytes, event) in midi.tracks[0].iter() {
        match event.kind {
            midly::TrackEventKind::Midi {
                channel: _,
                message,
            } => match message {
                midly::MidiMessage::NoteOn { .. } | midly::MidiMessage::NoteOff { .. } => {
                    conn_out.send(bytes).unwrap();
                    std::thread::sleep(Duration::from_secs(1));
                }
                _ => {}
            },
            _ => {}
        }
    }

    Json(())
}

#[derive(serde::Serialize)]
struct MidiSourceLoaded {
    file_name: String,
    format: String,
    tracks_count: usize,
    uuid: String,
}

async fn midi_add_handler(
    midi_sources: Extension<Arc<RwLock<MidiSources>>>,
    mut multipart: Multipart,
) -> Json<Vec<Result<MidiSourceLoaded, String>>> {
    let mut statuses = vec![];

    while let Some(field) = multipart.next_field().await.unwrap() {
        // TODO: Check that this is the name that we are expecting
        let _name = field.name().unwrap().to_string();
        // TODO: Better default file name
        let file_name = field.file_name().unwrap_or("<unknown>").to_string();
        let data = field.bytes().await.unwrap().to_vec();
        let uuid = uuid::Uuid::new_v4().to_string();

        let midi_source = MidiSource {
            bytes: data,
            file_name: file_name.clone(),
        };

        statuses.push(match midi_source.midi() {
            Ok(midi) => {
                let result = Ok(MidiSourceLoaded {
                    file_name,
                    format: match midi.header.format {
                        midly::Format::SingleTrack | midly::Format::Sequential => "sequential",
                        midly::Format::Parallel => "parallel",
                    }
                    .to_string(),
                    tracks_count: midi.tracks.len(),
                    uuid: uuid.clone(),
                });
                let midi_sources = &mut midi_sources.write().unwrap();
                midi_sources.insert(uuid, midi_source);
                result
            }
            Err(err) => Err(err.to_string()),
        });
    }
    Json(statuses)
}

// For expanding this websocket buisness see: https://github.com/tokio-rs/axum/blob/main/examples/websockets/src/main.rs
async fn websocket_handler(
    ws: WebSocketUpgrade,
    user_agent: Option<TypedHeader<headers::UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> impl IntoResponse {
    let user_agent = if let Some(TypedHeader(user_agent)) = user_agent {
        user_agent.to_string()
    } else {
        "unknown user agent".to_string()
    };
    info!("websocket connect: addr={addr}, user_agent={user_agent}");
    ws.on_upgrade(move |socket| handle_socket(socket, addr))
}

async fn handle_socket(mut socket: WebSocket, addr: SocketAddr) {
    if socket
        .send(Message::Text("hello, world".to_string()))
        .await
        .is_ok()
    {
        info!("websocket send to {addr} message: ");
    } else {
        return;
    }

    while let Some(msg) = socket.recv().await {
        let msg = match msg {
            Ok(msg) => msg,
            Err(err) => {
                error!("failed to receive: {err}");
                continue;
            }
        };
        match msg {
            Message::Text(msg) => println!("Received message: {msg}"),
            Message::Binary(bin) => println!("Binary message of length: {}", bin.len()),
            Message::Close(_) => return,
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }
}
