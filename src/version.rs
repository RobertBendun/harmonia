//! Extended version information of current Harmonia build
//!
//! Contains more information then typical version string, to make error reporting easier for
//! developer. See [Version] for full list of stored information.

use maud::{html, Markup};

/// Full information about current Harmonia build
pub struct Version {
    /// Version of package, reported in Cargo.toml
    pkg_version: &'static str,

    /// Short hash of commit pointed by HEAD in git
    hash: &'static str,

    /// Full hash of commit pointed by HEAD in git
    full_hash: &'static str,

    /// Local date of binary build
    date: &'static str,

    /// The state of repository during the build
    ///
    /// dirty = repository contained not committed changes
    dirty: &'static str,
}

impl Default for Version {
    /// Construct full Version information from information passed by `src/build.rs`
    fn default() -> Self {
        Self {
            pkg_version: env!("CARGO_PKG_VERSION"),
            hash: env!("GIT_STATUS_HASH"),
            full_hash: env!("GIT_STATUS_FULL_HASH"),
            date: build_time::build_time_local!("%Y-%m-%d %H:%M"),
            dirty: {
                let dirty = env!("GIT_STATUS_DIRTY");
                if dirty == "dirty" {
                    " dirty"
                } else {
                    ""
                }
            },
        }
    }
}

/// Pretty print Version information in terminal
impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            pkg_version,
            hash,
            date,
            dirty,
            ..
        } = self;
        write!(f, "{pkg_version} ({hash} {date}{dirty})")
    }
}

/// Pretty print version information in HTML
impl maud::Render for Version {
    fn render(&self) -> Markup {
        let Self {
            pkg_version,
            hash,
            full_hash,
            date,
            dirty,
        } = self;
        html! {
            (pkg_version);
            " (";
            a href=(format!("https://github.com/RobertBendun/harmonia/tree/{full_hash}")) {
                (hash);
            }
            " "; (date); (dirty); ")";
        }
    }
}
