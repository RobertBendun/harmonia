# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.0] - 2025-03-23

### Changed

- Bump required macOS version to 13 due to Github disabling macos-12 runner: https://github.com/actions/runner-images/issues/10721


## [0.6.0] - 2025-03-22

### Added

- Interface improvements: message on empty blocks list, red background on delete mode button, "title" attributes that describe inputs. Harmonia should be as selve descriptive as possible
- Enable Tokio multithreading
- Refresh network interfaces list (allows to change networks when using Harmonia!)
- Remove broken connections

### Changed

- Update axum to 0.8.1 (from 0.6.20)

### Fixed

- Don't log all MIDI messages in the terminal by default (they are still available as DEBUG messages)
- Better Multipart parsing during MIDI upload
- Showing playing status even when run without `linky_groups`

## [0.5.0] - 2024-11-15

### Added

- Nicks, saved in seperate file from blocks state

### Fixed

- Blocking refresh due to wrong usage of RWLock's

## [0.4.2] - 2024-11-15

### Added

- Abort button available only for localhost

## [0.4.0] - 2024-10-25

Additional release since Windows Defender marked previous one as trojan :)

### Added

- Press space to interrupt!

## [0.3.1] - 2024-10-25

### Fixed

- Connection to MIDI port 0 on Windows doesn't result in crash anymore
- When creating new MIDI source use minimal port number by default

## [0.3.0] - 2024-10-22

### Added

- Create new virtual MIDI port on start on Linux and macOS
- Enable ANSI processing for Windows cmd.exe
- Report multicast interfaces binds
- Enforce documentation in source code
- Enforce minimum MIDI port in UI

### Changed

- Allow reusing port number for `linky_groups` multicast connections (`SO_REUSEPORT`)

### Fixed

- `linky_groups` are no longer active when `--disable-link` parameter is passed

## [0.2.3] - 2024-08-07

### Changed

- Dependencies version to fix bugs on macOS

## [0.2.2] - 2024-06-14

### Added

- Print currently playing information in simple form

## [0.2.1] - 2024-06-01

### Added

- Developer documentation
- Multi machine Harmonia testing plan
- Connection to R&D project at AMU Poznań

### Fixed

- Wrong path prevented port number from beeing set for MIDI block

## [0.2.0] - 2024-05-31

### Added

- New shared memory API
- Example of shared memory API usage in Lua using C library
- Github CI tests pull requests

### Changed

- Introduced completely new "modern" UI
- Renamed "MIDI Sources" to "Blocks" to account for new forms of musical actions
- Declared support for macOS as experimental due to limited testing

## [0.1.2] - 2024-04-26

### Added

- Home path abbreviation for config path (somewhat privacy oriented change)
- Show number of peers in current state

### Fixed

- Refresh MIDI only refreshed the list - all the port associations stayed the same

## [0.1.1] - 2024-04-15

### Added

- Automatic Github Workflow that _should_ automatically build releases for supported platforms

### Fixed

- Stopping and switching played sources

## [0.1.0] - 2024-04-12

Initial version that was tested with Lambda Ensamble.
