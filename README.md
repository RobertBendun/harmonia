# Harmonia

__This project is currently beeing developed. It is not recommended to use it now. Wait for first release__

Harmonia is an interface for playing synchronized MIDI using modified [Ableton/Link](https://github.com/Ableton/link), dedicated for laptop orchestra.

## Usage

Go to [releases page](https://github.com/RobertBendun/harmonia/releases) and download the newest one for your operating system or build and run yourself:

```console
cargo run
```

Then go to the address pointed by the Harmonia binary (by default: [`http://localhost:8080`](http://localhost:8080).

See section "Compiling" below for instructions for your operating system.

Remember to allow any traffic from Harmonia in your firewall.

You can test it using [TEST-PLAN.md](./TEST-PLAN.md).
This is currently also the instruction on how to use Harmonia.

Developer documentation can be generated and viewed using `cargo`:

```console
cargo doc --open --document-private-items --bin harmonia --lib
```

### Compiling

Ubuntu 22.04 users, install this packages first:

```
apt install -y libasound2-dev cargo pkg-config cmake g++ clang git
```

Windows users, C++ capable Visual Studio, CMake and Rust toolchain is required.

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
- [ ] Name association (like nick) with Harmonia instance, by default hostname/username
- [ ] Harmonia as mDNS service
- [ ] Audio playing (wav, ogg itd)
- [ ] SonicPI integration

## Support for different platforms

All functionality should be working, tested regularly:

* Arch Linux,
* Ubuntu 22.04,
* Windows 11.

Experimental support:

* macOS,
* other Linux distributions (compilation on target system recommended).

Currently the main blocker for full macOS support is lack of resources - to properly test Harmonia at least two ARM macOS laptops would be required that are still supported by Apple.

As for Linux distribution, the diversity of sound systems (and GLIBC versions) require further research.

## Why Rust?

Since this library will be used with [Ableton/Link](https://github.com/Ableton/link) why write it in Rust?
On the one hand I tried writing web interface using C++ (and [Boost.Beast](https://github.com/boostorg/beast)) but UX of this library is painful.
Due to C++ releasing coroutines in C++20, and people pleaser mentality of the library it has broken API, supporting many different ways
of doing the same thing, which leads to noisy code, poor documentation and chaos.
Maybe I'm just not intended target or I'm not experienced enough with it or boost in general.

On the other hand, writing servers in Rust is almost-painless, with great documentation and API design.

## Patched libraries used by this project

- [Rust wrapper for Ableton Link](https://github.com/RobertBendun/rusty_link), [original repository](https://github.com/anzbert/rusty_link)
- [Ableton Link](https://github.com/RobertBendun/link), [original repository](https://github.com/Ableton/link)

They are referenced in [Cargo.toml](Cargo.toml) and [Cargo.lock](Cargo.lock) (recursivly, Rust wrapper references patched Ableton Link) files with links to the patched repositories.

Patching is required to break abstraction inside Ableton Link and don't reimplement it just to expose hidden state.
Further Harmonia versions may leave Link for custom solution but for now patched Link fulfills requirements perfectly.

## Harmonia as R&D on AMU University

See [uam/README.md](uam/README.md).

## Acknowledgements

This project couldn't become what it is without support and mentorship from:

- prof. UAM dr hab. Michał Hanćkowiak,
- prof. UAM dr hab. Maciej Grześkowiak,
- prof. UAM dr hab. Jacek Marciniak,
- and Lambda Ensamble laptop orchestra.

