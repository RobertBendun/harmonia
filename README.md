# Harmonia

__This project is currently beeing developed. It is not recommended to use it now. Wait for first release__

Harmonia is an interface for playing synchronized MIDI using [Ableton/Link](https://github.com/Ableton/link).

## Todos

Sorted by their completion & importance (if not completed yet).

- [x] Web server
- [x] MIDI upload
- [x] MIDI source download
- [x] Health check
- [x] List IPs (and maybe local names) listing
- [x] Server state save & restore - doesn't require user to provide all files every time
- [x] MIDI play subsystem with Ableton/Link
- [x] Keybindings for easy play functionality
- [x] Visual indicator which track is playing
- [ ] Name association (like nick) with harmonia instance, by default hostname/username
- [ ] Harmonia as mDNS service
- [ ] Audio playing (wav, ogg itd)
- [ ] SonicPI integration

## Why Rust?

Since this library will be used with [Ableton/Link](https://github.com/Ableton/link) why write it in Rust?
On the one hand I tried writing web interface using C++ (and [Boost.Beast](https://github.com/boostorg/beast)) but UX of this library is painful.
Due to C++ releasing coroutines in C++20, and people pleaser mentality of the library it has broken API, supporting many different ways
of doing the same thing, which leads to noisy code, poor documentation and chaos.
Maybe I'm just not intended target or I'm not expirienced enough with it or boost in general.

On the other hand, writing servers in Rust is almost-painless, with great documentation and API design.
