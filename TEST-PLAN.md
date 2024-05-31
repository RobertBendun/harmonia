# Test Plan

Below are a set of user interactions that are expected to work across all supported platforms running Harmonia instance.
Since automated testing for Harmonia is hard due to it's decentralized nature and heavy reliance on operating system any major changes are required to go through manual testing procedure to ensure quality.

## Common knowladge

### Common terms

* beat - monotonic clock that gets reset at shared start
* peers - number of Ableton Link (≈ Harmonia) instances
* keybind - keyboard shortcut set in keybind input for given block. When key is pressed, given block will play
* group - subgroup of synchronization session indicated by shared identifier (value of group input) that starts at the same time and wants to perform synchronously
* block - abstraction for anything that Harmonia can synchronously play (for example MIDI files) with associated metadata including group, keybind and additional data requried by format (for MIDI, MIDI port number)

### Interface layout

Interface layout ascii art below assumes widescreen.
However, Harmonia is responsive and elements of interface may collapse to single line.

```
Harmonia                   Version and storage
----------------------------------------------
Status      | List of blocks
            |
            |
            |
            |
            |
            |
            |
            |
            |---------------------------------
New MIDI    | Midi Outputs (collapsible)
New SHM     |---------------------------------
Delete mode | System information (collapsible)
----------------------------------------------
Interrupt |
          |
```

* `Status` consists of list of elements that show current status of synchronization system including:
    * `Synchronized` or `Error` which show connection status of Harmonia UI to the backend
    * `Peers` showing how many other Ableton/Link instances (≈ Harmonia instances) are in the network
    * `Beat` showing current beat value
    * `BPM` showing current BPM value
* `Version and storage` show backend version, including link to the specific commit that the application was compiled from and date of compilation and path to the cache of Harmonia data
* `List of blocks`, potentialy empty list of blocks - MIDI files, audio files, shared memory paths with controls to play, associate keybinding, group (and port for MIDI)
* `Midi Outputs` - list of MIDI outputs (source of numbers for port settings in blocks)
* `System information` - current computer name and IP address, useful for mDNS (not implemented yet) and file sharing

## Single-machine tests

This tests are being performed to ensure that Harmonia works on single machine.
This section must be completed before multiple-machine one, since state from this section will be used later.

### Linux setup

Performed mostly on Arch Linux, may work on any Linux distribution with ALSA.

Prerequisites:

* Running [Helm synth](https://tytel.org/helm/), connected to default MIDI port (or other synth.)
* Running local Harmonia instance
* Harmonia UI opened in the browser (default address is [http://localhost:8080](http://localhost:8080))
* Harmonia repository in the known path (for picking test MIDI files)

All the test cases must be done in order, none of them are optional.
Port used in tests is default MIDI port created by ALSA.

### Windows 11 setup

Prerequisites:

* [loopMIDI](https://www.tobias-erichsen.de/software/loopmidi.html) or other system to create virtual ports in Windows, with created virtual midi port for Harmonia - Helm communication
* [Helm synth](https://tytel.org/helm/) or other synth; should be connected to the loopMIDI virtual port.
* Running local Harmonia instance (not restricted by firewall)
* Harmonia UI opened in the browser (recommended: Firefox)
* Harmonia repository in the known path (for picking test MIDI files)

Windows doesn't support virtual MIDI ports out of the box and thus doesn't have default virtual MIDI port.
Instead it has builtin software synthesiser that is not recommended for any use due to non deterministic delays that builtin synth. introduces.

### Test Case: Adding new MIDI file

1. Ensure that list of blocks is empty (no blocks listed, deletion is shown in _Test Case: Delete Mode_)
1. Ensure that status is "Synchronized" in top left corner
1. Click "New MIDI" in left bottom corner
1. From OS file picker pick all MIDI files (`.mid` extension) located in "tests" directory inside Harmonia repository
1. Ensure that you have all distinct blocks (filenames + controls) listed
1. Download one of the files by clicking on the link with desired filename. Ensure that the checksum of downloaded file matches one in "tests" directory inside Harmonia repository

### Test Case: Playing MIDI source

1. Ensure that list of blocks is non-empty (adding blocks is shown in _Test Case: Adding new MIDI file_)
1. Ensure that status is "Synchronized" in top left corner
1. Ensure that number of peers is equal to 0. If number of pears is greater then 0 then kill all other Harmonia and Ableton/Link (for example Musique, Ableton Live) instances in your network.
1. On Windows, change port number to one matching your loopMIDI virtual port. Association of port numbers and port names can be found in collapsible MIDI Outputs panel.
1. Press play button for one of the blocks (triangle next to the filename). Ensure that synth. is receiving MIDI messages.
1. Press interrupt button (bottom left pause character). Ensure that synth. no longer produces sound. Harmonia should cleanup any ongoing notes.
1. Press play button again and after 2 seconds press play button of another block. Harmonia should immediately switch and start playing new block.
1. Interrupt using interrupt button.

### Test Case: Delete Mode

1. Ensure that list of blocks is non-empty (adding blocks is shown in _Test Case: Adding new MIDI file_)
1. Ensure that status is "Synchronized" in top left corner
1. Click "Delete mode" in bottom left corner, delete icons (red background, basket emoji) should be shown
1. Delete one of the blocks using delete button (red background, basket emoji)
1. Ensure that the deleted item is not shown on the list
1. Click "Delete mode" button again, delete icons should hide
1. Ensure that after page reload the deleted item is not shown on the list


