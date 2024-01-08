use maud::{html, Markup};

pub struct Version {
    pkg_version: &'static str,
    hash: &'static str,
    full_hash: &'static str,
    date: &'static str,
    dirty: &'static str,
}

impl Default for Version {
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
