//! Routes used in router setup in [crate::main].
//!
//! Middle man between UI, [audio_engine] and synchronization mechanisms.
//!
//! All of the HTTP routes (without WebSockets) are located in this module.
//! Handlers return HTML using [maud] templating library. Some functions are partials that are only
//! used inside this module, some are exposed to the [axum] router to be accessible by [HTMX].
//!
//! [HTMX]: https://htmx.org
//!
//! Structs inside this file are only schemas for HTTP forms.

// TODO:  This triplets of {SetX, midi_set_x_for_source, render_x_cell} maybe should be
// consolidated

use crate::{audio_engine, block, cache_path, AppState, Version};
use axum::{
    extract::{ConnectInfo, Path, State},
    http,
    response::IntoResponse,
    Form,
};
use axum_extra::extract::Multipart;
use maud::{html, Markup, PreEscaped, DOCTYPE};
use midir::MidiOutput;
use rusty_link::SessionState;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tracing::{error, info};

/// Name used by the `<input type="file">` for MIDI upload
const MIDI_FILE_INPUT_FORM_NAME: &str = "midi";

/// Main route, "/" handler, renders whole interface as HTML
pub async fn index(
    addr: ConnectInfo<crate::SocketAddr>,
    app_state: State<Arc<AppState>>,
) -> Markup {
    html! {
        (DOCTYPE);
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "Harmonia" }
                meta name="viewport" content="width=device-width, initial-scale=1";
                script src="index.js" {}
                script src="htmx.min.js" {}
                link rel="stylesheet" href="index.css";
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
                    h1 { "Harmonia" }
                    div {
                        div {
                            "Version: ";
                            (Version::default());
                        }
                        div {
                            "Data: ";
                            ({
                                let cache = cache_path();
                                let c: &std::path::Path = &cache;

                                dirs::home_dir()
                                    .and_then(|home| c.strip_prefix(home).ok())
                                    .map(|p| PathBuf::from(if cfg!(unix) { "~" } else { "%HOMEPATH%" }).join(p))
                                    .unwrap_or(cache)
                                    .to_str()
                                    .unwrap()
                                    .to_owned()
                            });
                        }
                    }
                }

                aside {
                    (runtime_status(app_state.clone()).await);
                    div {
                        label for="midi" { "New MIDI" }
                        input
                            style="display: none"
                            type="file"
                            id="midi"
                            name=(MIDI_FILE_INPUT_FORM_NAME)
                            multiple
                            accept="audio/midi"
                            hx-put="/blocks/midi"
                            hx-target="#blocks"
                            hx-swap="innerHTML"
                            hx-encoding="multipart/form-data";
                        button onclick="toggle_delete(this)" {
                            "Delete mode"
                        }
                    }
                }

                main id="blocks" {
                    (blocks(app_state.clone()).await)
                }

                details class="midi-outputs" {
                    summary { "MIDI Outputs" }
                    p {
                        "Refresh page (F5 or CTRL-R) to refresh list of available MIDI ports";
                    }
                    (midi_ports(app_state.clone()).await)
                }

                details class="system-information" {
                    summary { "System information" }
                    (system_information(app_state.clone()).await);
                    @if addr.ip().is_loopback() {
                        button hx-post="/abort" hx-confirm="Are you sure that you want to close Harmonia?"  {
                            "Abort Harmonia instance"
                        }
                    }
                }

                footer {
                    button
                        hx-post="/interrupt"
                        hx-swap="none"
                        style="grid-area: stop"
                    {
                        (PreEscaped("&#x23f8;"))
                    }
                    (playing_status(app_state.clone()).await)
                }
            }
        }
    }
}

/// Renders synchronization state, including current time (beats)
pub async fn runtime_status(app_state: State<Arc<AppState>>) -> Markup {
    let mut session_state = SessionState::default();
    let active = app_state.link.is_enabled();
    app_state.link.capture_app_session_state(&mut session_state);
    let time = app_state.link.clock_micros();

    // TODO: Move quantum to state
    let quantum = 1.0;
    let beat = session_state.beat_at_time(time, quantum);
    let peers = app_state.link.num_peers();

    html! {
        table id="status" {
            tr {
                th title="" id="app-health" colspan="2" {
                    @if active { "Synchronized" }
                    @else { "ERROR" }
                }
            }
            tr { th title="How many other person you see" { "Peers" } td { (peers) } }
            tr { th { "Beat" } td { (format!("{beat:.1}")) } }
            tr { th { "BPM" } td { (session_state.tempo()) } }
        }
    }
}

/// Renders playing state or nothing (if nothing is played)
pub async fn playing_status(app_state: State<Arc<AppState>>) -> Markup {
    let currently_playing_uuid = app_state.currently_playing_uuid.read().unwrap();
    let current_playing_progress = app_state.current_playing_progress.read().unwrap();

    let is_infinite =
        current_playing_progress.0 == current_playing_progress.1 && current_playing_progress.0 == 0;

    html! {
        div id="playing-status" {
            @if currently_playing_uuid.is_some() {
                @if is_infinite {
                    div class="progress infinite" {
                        div style="height: 100%; background-color: gray" {}
                        (maud::PreEscaped("&#x221E;"));
                    }
                } @else {
                    div class="progress" {
                        div style="height: 100%; background-color: gray" {}
                        (format!("{}%", current_playing_progress.0 * 100 / current_playing_progress.1));
                    }
                }
                div style="grid-are: info" {
                    ({
                        if let Some(uuid) = currently_playing_uuid.as_ref() {
                            let blocks = app_state.blocks.read().unwrap();
                            if let Some(block) = blocks.get(uuid) {
                                match &block.content {
                                    block::Content::Midi(m) => m.file_name.clone(),
                                    block::Content::SharedMemory { path } => path.to_string(),
                                }
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        }
                    })
                }
            }
        }
    }
}

/// Renders information about the system
///
/// Since Harmonia is by default accessible on all interfaces, a convenient way to use share files
/// is to ask colleague what IP address they have and quickly enter their instance in browser
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

    let hostname = whoami::devicename();
    let nick = app_state.nick.read().await;

    html! {
        p {
            "Hostname: "; a href=(format!("http://{hostname}:{port}")) {
                (hostname)
            }
        }
        p {
            label for="nick" { "Nick: " }
            input type="text" name="nick" value=(nick) hx-post="/nick" title="Name that can be used to identify yourself" autocomplete="off";

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

/// Render list of currently held ports in [AppState]
pub async fn midi_ports(State(app_state): State<Arc<AppState>>) -> Markup {
    let out = MidiOutput::new("harmonia").unwrap();

    if let Ok(mut midi_conn) = app_state.connection.try_write() {
        midi_conn.refresh();
    }

    let midi_conn = app_state.connection.read().unwrap();

    let ports = midi_conn
        .ports
        .iter()
        .filter_map(|port| Result::ok(out.port_name(port)));

    html! {
        ol start=(if cfg!(unix) { 0 } else { 1 }) {
            @if cfg!(unix) {
                li {
                    "Builtin Harmonia MIDI Virtual Port"
                }
            }

            @for port_name in ports {
                li { (port_name) }
            }
        }
    }
}

/// Render currently held blocks
async fn blocks(app_state: State<Arc<AppState>>) -> Markup {
    use crate::block::Content;

    let blocks = app_state.blocks.read().unwrap();
    if blocks.is_empty() {
        return html! {
            p style="text-align: center" {
                "You can add MIDI files by pressing „New MIDI” in the bottom left corner. ";
                "You can select multiple files.";
                br;
                "After selection, they will appear here. Associate each one with output port and group to enable synchronization.";
            }
        };
    }

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
            section class="block" {
                button
                    class="delete-mode icon-control"
                    hx-delete=(format!("/blocks/{uuid}"))
                    hx-swap="innerHTML"
                    hx-target="#blocks"
                {
                    "🗑️"
                }
                button
                    hx-post=(format!("/blocks/play/{uuid}"))
                    hx-swap="none"
                    class="icon-control"
                {
                    "▶"
                }
                div {
                    @match &block.content {
                        Content::Midi(source) => {
                            a href=(format!("/blocks/{uuid}")) { (source.file_name) }
                        }
                        Content::SharedMemory { path } => (path),
                    }
                }

                @if let Content::Midi(source) = &block.content {
                    (port_cell(uuid, source.associated_port))
                }

                (group(uuid, &block.group));
                (keybind(uuid, &block.keybind));
            }
        }
    }
}

/// Render group input
fn group(uuid: &str, group: &str) -> Markup {
    html! {
        input
            type="text" value=(group)
            pattern=(format!("(\\w| ){{0,{}}}", linky_groups::MAX_GROUP_ID_LENGTH))
            maxlength=(linky_groups::MAX_GROUP_ID_LENGTH)
            name="group"
            placeholder="Group"
            hx-target="this"
            hx-post=(format!("/blocks/set-group/{uuid}"))
            autocomplete="off"
            title="Label which identifies with whom synchronize. Computers with same label here will be synchronized.";
    }
}

/// Schema for setting group for block request
#[derive(Deserialize)]
pub struct SetGroup {
    /// Group identifier to set
    pub group: String,
}

/// Set group for given block
pub async fn set_group(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
    Form(SetGroup {
        group: mut group_to_set,
    }): Form<SetGroup>,
) -> Result<Markup, http::StatusCode> {
    let response = {
        let mut blocks = app_state.blocks.write().unwrap();

        let Some(midi_source) = blocks.get_mut(&uuid) else {
            error!("block#{uuid} not found");
            return Err(http::StatusCode::NOT_FOUND);
        };

        if group_to_set.len() > linky_groups::MAX_GROUP_ID_LENGTH {
            let mut cut = linky_groups::MAX_GROUP_ID_LENGTH;
            while !group_to_set.is_char_boundary(cut) {
                cut -= 1;
            }
            tracing::warn!(
                "provided group {:?} is longer then allowed ({} bytes), truncating to {:?}",
                &group_to_set,
                linky_groups::MAX_GROUP_ID_LENGTH,
                &group_to_set[..cut]
            );
            group_to_set.truncate(cut);
        }
        midi_source.group = group_to_set;

        tracing::info!(
            "Switched block#{uuid} to group {group:?}",
            group = midi_source.group
        );
        Ok(group(&uuid, &midi_source.group))
    };

    if let Err(err) = app_state.remember_current_blocks() {
        error!("block_set_group failed to remember current sources: {err:#}")
    }
    response
}

/// Render current keybind for block in input form
fn keybind(uuid: &str, keybind: &str) -> Markup {
    html! {
        input
            name="keybind"
            data-uuid=(uuid)
            onchange="update_key_binding(this)"
            type="text"
            hx-post=(format!("/blocks/set-keybind/{uuid}"))
            hx-swap="none"
            placeholder="Keybind"
            value=(keybind)
            autocomplete="off"
            title="Associate key that after press will start this block.";
    }
}

/// Schema for request that sets keybind for given block
#[derive(Deserialize)]
pub struct SetKeybind {
    /// Kebind to set
    pub keybind: String,
}

/// Sets keybind for given block
pub async fn set_keybind(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
    Form(SetKeybind { keybind }): Form<SetKeybind>,
) -> http::StatusCode {
    {
        let mut midi_sources = app_state.blocks.write().unwrap();

        let Some(block) = midi_sources.get_mut(&uuid) else {
            error!("block#{uuid} not found");
            return http::StatusCode::NOT_FOUND;
        };

        info!("Changing keybind for block#{uuid} to {keybind}");
        block.keybind = keybind;
    }

    if let Err(err) = app_state.remember_current_blocks() {
        error!("set_keybind failed to remember current sources: {err:#}")
    }

    http::StatusCode::OK
}

// TODO: Should be select
// TODO: max should be dynamic
/// Renders port input for MIDI port
fn port_cell(uuid: &str, associated_port: usize) -> Markup {
    html! {
        input
            type="number" value=(format!("{}", associated_port))
            name="port"
            min=(MIN_PORT_NUMBER)
            hx-target="this"
            hx-swap="outerHTML"
            hx-post=(format!("/blocks/midi/set-port/{uuid}"))
            autocomplete="off"
            title="Port number to send MIDI to. List of ports available below in „MIDI Outputs”";
    }
}

/// Schema for port selection for block containing MIDI
#[derive(Deserialize)]
pub struct SetPort {
    /// MIDI port to set for Midi block
    pub port: usize,
}

/// Minimum MIDI port number
///
/// On windows it is 1 since we don't have virtual ports.
/// On unix'es it's 0 since we have virtual ports.
pub const MIN_PORT_NUMBER: usize = if cfg!(unix) { 0 } else { 1 };

/// Set port for MIDI block
pub async fn set_port_for_midi(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
    Form(SetPort { port }): Form<SetPort>,
) -> Result<Markup, http::StatusCode> {
    let mut blocks = app_state.blocks.write().unwrap();

    let Some(block) = blocks.get_mut(&uuid) else {
        error!("block#{uuid} was not found");
        return Err(http::StatusCode::NOT_FOUND);
    };

    let block::Content::Midi(ref mut midi) = block.content else {
        error!("block#{uuid} is not a MIDI source");
        return Err(http::StatusCode::BAD_REQUEST);
    };

    let max = app_state.connection.read().unwrap().ports.len();

    #[allow(clippy::absurd_extreme_comparisons)]
    if port < MIN_PORT_NUMBER || port > max {
        error!("port number should be between {MIN_PORT_NUMBER} and {max}");
        return Ok(port_cell(&uuid, midi.associated_port));
    }

    info!("setting port {port} for {uuid}");
    midi.associated_port = port;
    Ok(port_cell(&uuid, midi.associated_port))
}

/// Responds with content of block if block had any
pub async fn download_block_content(
    app_state: State<Arc<AppState>>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    let not_found = || {
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::CONTENT_TYPE, "text/plain".parse().unwrap());
        (
            http::StatusCode::NOT_FOUND,
            headers,
            "not found".bytes().collect(),
        )
    };

    let blocks = app_state.blocks.read().unwrap();
    let Some(block) = blocks.get(&uuid) else {
        error!("block#{uuid} not found");
        return not_found();
    };

    match &block.content {
        block::Content::SharedMemory { .. } => not_found(),
        block::Content::Midi(midi_source) => {
            let mut headers = http::HeaderMap::new();
            headers.insert(
                http::header::CONTENT_DISPOSITION,
                format!("attachement; filename=\"{}\"", midi_source.file_name)
                    .parse()
                    .unwrap(),
            );
            headers.insert(http::header::CONTENT_TYPE, "audio/midi".parse().unwrap());
            (http::StatusCode::OK, headers, midi_source.bytes.clone())
        }
    }
}

/// Removes block based on ID and caches currently held blocks
pub async fn remove_block(app_state: State<Arc<AppState>>, Path(uuid): Path<String>) -> Markup {
    {
        let mut sources = app_state.blocks.write().unwrap();
        sources.remove(&uuid);
    }
    if let Err(err) = app_state.remember_current_blocks() {
        error!("remove_midi_source_handler failed to remember current sources: {err:#}")
    }

    blocks(app_state).await
}

/// Starts playing given block
pub async fn play(State(app_state): State<Arc<AppState>>, Path(uuid): Path<String>) {
    let _ = audio_engine::play(app_state.clone(), &uuid).await;
}

/// Interrupts any currently played block (or does nothing)
pub async fn interrupt(State(app_state): State<Arc<AppState>>) {
    if let Err(error) = audio_engine::interrupt(app_state).await {
        tracing::error!("failed to interrupt: {error}");
    }
}

/// Schema for creation of new shared memory block
#[derive(Deserialize)]
pub struct AddSharedMemoryBlock {
    /// Path to shared memory block
    path: String,
}

/// Add new shared memory block and cache list of blocks
pub async fn add_new_shered_memory_block(
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

    blocks(axum::extract::State(app_state)).await
}

/// Adds new MIDI block(s) based on the provided files in HTML Form
#[axum::debug_handler]
pub async fn add_new_midi_source_block(
    State(app_state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Markup {
    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap();
        if name != MIDI_FILE_INPUT_FORM_NAME {
            tracing::warn!("Found unknown field in multipart request: {}", name);
            continue;
        }

        let file_name = field.file_name().unwrap_or("<unknown>").to_string();
        let data = field.bytes().await.unwrap().to_vec();
        let mut hasher = Sha1::new();
        hasher.update(&data);
        let uuid = hex::encode(hasher.finalize());

        let midi_source = block::MidiSource {
            bytes: data,
            file_name,
            associated_port: MIN_PORT_NUMBER,
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

    blocks(axum::extract::State(app_state)).await
}

/// Abort application on user's request
///
/// Note that application can be stopped only from localhost
pub async fn abort(
    addr: ConnectInfo<crate::SocketAddr>,
    app_state: State<Arc<AppState>>,
) -> http::HeaderMap {
    let mut headers = http::HeaderMap::new();

    if addr.ip().is_loopback() {
        app_state.abort.notify_one();
        headers.insert("HX-Redirect", "/".parse().unwrap());
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    headers
}

/// Payload to set a nick
#[derive(Deserialize)]
pub struct SetNick {
    /// Name that user prefers
    nick: String,
}

pub async fn nick(app_state: State<Arc<AppState>>) -> String {
    app_state.nick.read().await.clone()
}

/// Set nick and save it to file
pub async fn set_nick(app_state: State<Arc<AppState>>, Form(SetNick { nick }): Form<SetNick>) {
    let mut nick_ref = app_state.nick.write().await;
    let nick = nick.trim();
    tracing::info!("setting nick to: {nick:?}");
    *nick_ref = nick.to_string();

    let nick_full_path = cache_path().join(crate::NICK_PATH);
    if let Err(error) = std::fs::write(&nick_full_path, nick) {
        tracing::warn!("failed to write nick to {nick_full_path:?}: {error}");
    }
}
