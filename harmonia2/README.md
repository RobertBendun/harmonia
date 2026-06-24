# Harmonia 2

Redesign of Harmonia project after over 2 years of production expirience.

Goals:

1. library is exposed as just algorithm, library consumers/wrappers introduce IO and execution context - this will hopefully allow easy adoption both as standalone applications and as plugins
2. library should work with midi/midi2/audio
3. direct usage of audio/midi API from operating systems
4. support for Linux (Pipewire/Jack/Alsa in that order), Windows (with new MIDI 2 support on Windows if possible), macOS
