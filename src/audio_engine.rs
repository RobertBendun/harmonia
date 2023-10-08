use std::{
    sync::{mpsc, Arc, Weak},
    thread::JoinHandle,
    time::Duration,
};

use midir::{MidiOutput, MidiOutputConnection};
use rusty_link::SessionState;
use tracing::{info, warn};

use crate::AppState;

pub struct AudioEngine {
    pub state: Weak<AppState>,
    // TODO: Sefely join and destroy this thread using some sort of contition variable
    // synchronization to break it's work and then join to wait until work is finished.
    #[allow(dead_code)]
    worker: JoinHandle<()>,
    work_in: mpsc::SyncSender<Job>,
}

struct Job {
    output: MidiOutputConnection,
    uuid: String,
    app_state: Arc<AppState>,
}

// More info on timing:
// https://majicdesigns.github.io/MD_MIDIFile/page_timing.html
fn audio_engine_main(job: Job) -> Result<(), String> {
    let Job {
        mut output,
        uuid,
        app_state,
    } = job;

    // TODO: Find better solution then checking two times if we have this source
    let midi_sources = app_state.sources.read().unwrap();
    let Some(midi_source) = midi_sources.get(&uuid) else {
        return Err(format!("{uuid} not found. Try reloading page"));
    };

    let midi = midi_source
        .midi()
        .map_err(|err| format!("failed to parse midi: {err}"))?;

    let ticks_per_quater_note = match midi.header.timing {
        midly::Timing::Metrical(tpqn) => tpqn.as_int(),
        _ => return Err("Timecode timing format is not supported".to_string()),
    };

    let mut session_state = SessionState::new();
    let quantum = 4.0;

    info!("commiting start state");

    app_state.link.capture_app_session_state(&mut session_state);
    session_state.set_is_playing_and_request_beat_at_time(
        true,
        app_state.link.clock_micros() as u64,
        0.0,
        quantum,
    );
    app_state.link.commit_app_session_state(&session_state);

    let mut time_passed = 0.0;

    for (bytes, event) in midi.tracks[0].iter() {
        app_state.link.capture_app_session_state(&mut session_state);

        let time_to_wait = event.delta.as_int() as f64 / ticks_per_quater_note as f64;
        let current_time = session_state.beat_at_time(app_state.link.clock_micros(), quantum);
        time_passed += time_to_wait;

        info!("[passed={time_passed}, current={current_time}] {event:?}");
        if current_time < time_passed {
            let sleep_time = (time_passed - current_time) / 120.0 * 60.0;
            std::thread::sleep(Duration::from_secs_f64(sleep_time));
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
            midly::TrackEventKind::Midi {
                channel: _,
                message,
            } => match message {
                midly::MidiMessage::NoteOn { .. } | midly::MidiMessage::NoteOff { .. } => {
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

    Ok(())
}

impl Default for AudioEngine {
    fn default() -> Self {
        // TODO: We assume that we handle this request relativly quickly (and thus only 1 in queue)
        let (work_in, work) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            while let Ok(job) = work.recv() {
                if let Err(err) = audio_engine_main(job) {
                    crate::error!(err);
                }
            }
        });

        Self {
            state: Default::default(),
            worker,
            work_in,
        }
    }
}

pub async fn play(app_state: Arc<AppState>, uuid: &str) -> Result<(), String> {
    let midi_sources = app_state.sources.read().unwrap();
    let Some(midi_source) = midi_sources.get(uuid) else {
        return Err(format!("{uuid} not found. Try reloading page"));
    };

    midi_source
        .midi()
        .map_err(|err| format!("failed to parse midi: {err}"))?;

    let midi_out = MidiOutput::new("harmonia")
        .map_err(|err| format!("failed to create midi output port: {err}"))?;
    // TODO: Use associated port
    let midi_port = &midi_out.ports()[0];
    info!(
        "outputing to output port: {}",
        midi_out.port_name(midi_port).unwrap()
    );
    let conn_out = midi_out
        .connect(midi_port, /* TODO: Better name */ "play")
        .map_err(|err| format!("failed to connect to midi port: {err}"))?;

    app_state
        .audio_engine
        .write()
        .unwrap()
        .work_in
        .send(Job {
            output: conn_out,
            uuid: uuid.to_string(),
            app_state: app_state.clone(),
        })
        .map_err(|err| format!("failed to send job: {err}"))?;

    Ok(())
}
