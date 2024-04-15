use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use midir::MidiOutput;
use midly::live::LiveEvent;
use rusty_link::SessionState;
use tracing::{info, warn};

use crate::AppState;

pub struct AudioEngine {
    pub state: Weak<AppState>,
    // TODO: Sefely join and destroy this thread using some sort of contition variable
    // synchronization to break it's work and then join to wait until work is finished.
    worker: Option<tokio::task::JoinHandle<()>>,
    work_in: tokio::sync::mpsc::Sender<Request>,
}

#[derive(Debug)]
enum Request {
    Quit,
    Interrupt,
    Play(RequestPlay),
}

struct RequestPlay {
    uuid: String,
    app_state: Arc<AppState>,
}

impl std::fmt::Debug for RequestPlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RequestPlay {{ uuid: {uuid:?} }}", uuid = self.uuid)
    }
}

// More info on timing:
// https://majicdesigns.github.io/MD_MIDIFile/page_timing.html
async fn audio_engine_main(
    request_play: RequestPlay,
    interrupts: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
) -> Result<(), String> {
    let RequestPlay { uuid, app_state } = request_play;

    let midi_source = {
        let midi_sources = app_state.sources.read().unwrap();
        let Some(midi_source) = midi_sources.get(&uuid) else {
            return Err(format!("{uuid} not found. Try reloading page"));
        };
        midi_source.clone()
    };

    let mut output = {
        let out = MidiOutput::new("harmonia")
            .map_err(|error| format!("failed to open midi output: {error}"))?;
        let midi_port = &out.ports()[midi_source.associated_port];
        info!(
            "outputing to output port #{} named: {}",
            midi_source.associated_port,
            out.port_name(midi_port).unwrap(),
        );

        out.connect(midi_port, /* TODO: Better name */ "harmonia-play")
            .map_err(|err| format!("failed to connect to midi port: {err}"))?
    };

    let mut session_state = SessionState::new();
    let quantum = 1.0;

    if midi_source.group.is_empty() {
        tracing::info!("Empty group, starting using request_beat_at_time");
        app_state.link.capture_app_session_state(&mut session_state);
        session_state.request_beat_at_time(0.0, app_state.link.clock_micros(), quantum);
        app_state.link.commit_app_session_state(&session_state);
    } else {
        tracing::info!("Starting with group: {group:?}", group = midi_source.group);
        app_state
            .groups
            .as_ref()
            .unwrap()
            .start(&midi_source.group)
            .await
            .unwrap();
    }

    tokio::task::spawn_blocking(move || {
        // TODO: Find better solution then checking two times if we have this source
        let midi = midi_source
            .midi()
            .map_err(|err| format!("failed to parse midi: {err}"))
            .unwrap();

        let ticks_per_quater_note = match midi.header.timing {
            midly::Timing::Metrical(tpqn) => tpqn.as_int(),
            _ => {
                output.close();
                panic!("Timecode timing format is not supported");
            }
        };
        *app_state.currently_playing_uuid.write().unwrap() = Some(uuid.to_string());
        *app_state.current_playing_progress.write().unwrap() =
            (0_usize, midi.tracks.last().unwrap().len());
        info!("commiting start state");

        let mut time_passed = 0.0;
        let mut track = midi.tracks.last().unwrap().iter().enumerate();

        let mut notes_played_per_channel = [[false; 128]; 16];

        'audio_loop: loop {
            let Some((nth, (bytes, event))) = track.next() else {
                break;
            };

            *app_state.current_playing_progress.write().unwrap() =
                (nth, midi.tracks.last().unwrap().len());

            let (interrupt, interruptable_sleep) = &*interrupts;
            let interrupted = interrupt.try_lock().map(|x| *x).unwrap_or(false);
            if interrupted {
                break;
            }

            let time_to_wait = event.delta.as_int() as f64 / ticks_per_quater_note as f64;
            time_passed += time_to_wait;

            // TODO: Rust makes a note that condvar shouldn't be use in time critical applications?
            loop {
                app_state.link.capture_app_session_state(&mut session_state);
                let current_time =
                    session_state.beat_at_time(app_state.link.clock_micros(), quantum);

                info!("[passed={time_passed}, current={current_time}] {event:?}");
                if current_time >= time_passed {
                    break;
                }

                let sleep_time = (time_passed - current_time) / 120.0 * 60.0;
                let guard = interrupt.lock().unwrap();
                let (interrupted, sleep_result) = interruptable_sleep
                    .wait_timeout(guard, Duration::from_secs_f64(sleep_time))
                    .unwrap();
                if *interrupted {
                    break 'audio_loop;
                }
                if sleep_result.timed_out() {
                    break;
                }
            }

            match event.kind {
                midly::TrackEventKind::Meta(meta) => match meta {
                    // http://midi.teragonaudio.com/tech/midifile/ppqn.htm
                    midly::MetaMessage::Tempo(tempo) => {
                        let tempo: f32 = 60_000_000.0 / (tempo.as_int() as f32);
                        info!("changing tempo to {}", tempo)
                    }

                    // http://midi.teragonaudio.com/tech/midifile/time.htm
                    midly::MetaMessage::TimeSignature(num, den, _, _) => {
                        info!(
                            "time signature is: {num}/{den}",
                            den = 2_usize.pow(den.into())
                        )
                    }

                    // These are obligatory at the end of track so we don't need to handle them
                    midly::MetaMessage::EndOfTrack => {}
                    msg => {
                        warn!("unknown meta message: {msg:?}")
                    }
                },
                midly::TrackEventKind::Midi { channel, message } => match message {
                    // TODO: Remember currenlty played notes so we can unwind this musical stack when
                    // we turn it off.
                    midly::MidiMessage::NoteOn { key, vel } => {
                        notes_played_per_channel[channel.as_int() as usize]
                            [key.as_int() as usize] = vel != 0;
                        output.send(bytes).unwrap();
                    }
                    midly::MidiMessage::NoteOff { key, .. } => {
                        notes_played_per_channel[channel.as_int() as usize]
                            [key.as_int() as usize] = false;
                        output.send(bytes).unwrap();
                    }
                    msg => {
                        warn!("unknown midi message: {msg:?}")
                    }
                },
                midly::TrackEventKind::SysEx(_) => {
                    // TODO: They should probably be forwarded
                    warn!("sysex messages are not handled yet");
                }
                midly::TrackEventKind::Escape(_) => {
                    // TODO: They should probably be forwarded
                    warn!("escape messages are not handled yet");
                }
            }
        }

        let task = {
            let app_state = app_state.clone();
            tokio::spawn(async move { app_state.groups.as_ref().unwrap().stop().await })
        };

        let mut buf = Vec::new();
        for (channel, notes) in notes_played_per_channel.iter().enumerate() {
            for (key, played) in notes.iter().enumerate() {
                if *played {
                    let event = LiveEvent::Midi {
                        channel: (channel as u8).into(),
                        message: midly::MidiMessage::NoteOff {
                            key: (key as u8).into(),
                            vel: 0.into(),
                        },
                    };

                    event.write(&mut buf).expect(
                        "this notes were produced from valid MIDI messages so they must serialize",
                    );
                    if let Err(error) = output.send(&buf) {
                        tracing::error!("failed to send cleanup note off message: {error}");
                    }
                    buf.clear();
                }
            }
        }

        output.close();
        *app_state.currently_playing_uuid.write().unwrap() = None;
        task
    })
    .await
    .unwrap()
    .await
    .unwrap();
    Ok(())
}

impl Default for AudioEngine {
    fn default() -> Self {
        // TODO: We assume that we handle this request relativly quickly (and thus only 1 in queue)
        let (work_in, mut work) = tokio::sync::mpsc::channel(1);

        let mut interrupt: Option<Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>> = None;
        let mut current_worker: Option<tokio::task::JoinHandle<()>> = None;

        let worker = tokio::spawn(async move {
            while let Some(request) = work.recv().await {
                info!("received request: {request:?}");

                if let Some(interrupt) = interrupt.take() {
                    *interrupt.0.lock().unwrap() = true;
                    interrupt.1.notify_one();
                    if let Some(current_worker) = current_worker.take() {
                        current_worker.await.unwrap();
                    }
                }

                let request = match request {
                    Request::Play(request) => request,
                    Request::Interrupt => continue,
                    Request::Quit => break,
                };

                interrupt = Some(Arc::new((
                    std::sync::Mutex::new(false),
                    std::sync::Condvar::new(),
                )));
                let worker_interrupt = interrupt.clone().unwrap();
                let worker = tokio::spawn(async move {
                    if let Err(err) = audio_engine_main(request, worker_interrupt).await {
                        crate::error!("{err:#}")
                    }
                });
                current_worker = Some(worker);
            }
        });

        Self {
            state: Default::default(),
            worker: Some(worker),
            work_in,
        }
    }
}

pub async fn quit(app_state: Arc<AppState>) {
    let work_in = {
        let audio_engine = app_state.audio_engine.write().unwrap();
        audio_engine.work_in.clone()
    };

    if work_in.send(Request::Quit).await.is_err() {
        return;
    }

    let worker = app_state.audio_engine.write().unwrap().worker.take();
    if let Some(worker) = worker {
        let _ = worker.await;
    }
}

// TODO: Since axum is using tokio under the hood this should be compatible with http handlers
pub async fn interrupt(app_state: Arc<AppState>) -> Result<(), String> {
    let work_in = {
        let audio_engine = app_state.audio_engine.write().unwrap();
        audio_engine.work_in.clone()
    };

    work_in
        .send(Request::Interrupt)
        .await
        .map_err(|err| format!("failed to send job: {err}"))
}

pub async fn play(app_state: Arc<AppState>, uuid: &str) -> Result<(), String> {
    // TODO: This is wrong approach, we should select what will be played, not what to play now.
    let work_in = app_state.audio_engine.write().unwrap().work_in.clone();

    work_in
        .send(Request::Play(RequestPlay {
            uuid: uuid.to_string(),
            app_state: app_state.clone(),
        }))
        .await
        .map_err(|err| format!("failed to send job: {err}"))?;

    Ok(())
}
