# Example: simple Lua integration using unix sockets

This example creates a simple musical embeded language inside Lua, using coroutines.
It provides mechanism that is similar in UX to the very simplified SonicPI.

Tested only on Linux. Requires [RtMidi](https://github.com/thestk/rtmidi), [Lua 5.4](https://www.lua.org/) and C compiler.

## Usage

```console
$ make
$ lua5.4 demo.lua
```
