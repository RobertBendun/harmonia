//! Blocks definition - basic units of something playable in Harmonia
//!
//! Harmonia blocks are the basic building block of Harmonia execution.
//! They store the common state (group name, associated keybind and other metadata)
//! and specialized information about the thing that will be played. See [Content] for currently
//! playable types in Harmonia.

use serde::{Deserialize, Serialize};

/// Representation of anything that can be played with Harmonia
#[derive(Serialize, Deserialize, Clone)]
pub struct Block {
    /// Custom order from the user
    pub order: Option<usize>,

    /// Group in which given content will be played synchronously
    pub group: String,

    /// Associated user keybind if any
    pub keybind: String,

    /// Description of what and how will be played
    pub content: Content,
}

/// Different kinds of contents that can be played with Harmonia
///
/// This type is consumed in [audio_engine], produced in UI [handlers].
/// Here is only the description, the usage is defined in [audio_engine] implementation.
///
/// [audio_engine]: crate::audio_engine
/// [handlers]: crate::handlers
#[derive(Serialize, Deserialize, Clone)]
pub enum Content {
    /// MIDI file itself with metadata
    Midi(MidiSource),

    /// Path pointing to shared memory
    SharedMemory { path: String },
}

impl Content {
    /// Human readable name of given content
    pub fn name(&self) -> String {
        match self {
            Self::Midi(midi_source) => midi_source.file_name.clone(),
            Self::SharedMemory { path } => path.clone(),
        }
    }
}

/// Description of MIDI sources
#[derive(Serialize, Deserialize, Clone)]
pub struct MidiSource {
    /// MIDI source itself
    pub bytes: Vec<u8>,

    /// Original file name of MIDI source
    pub file_name: String,

    /// Refers to allocated MIDI ports list
    pub associated_port: usize,
}

impl MidiSource {
    pub fn midi(&self) -> Result<midly::SmfBytemap<'_>, midly::Error> {
        midly::SmfBytemap::parse(&self.bytes)
    }
}
