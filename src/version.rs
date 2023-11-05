use maud::{Markup, html};


// TODO: Consolidate this two functions somehow
//       The only difference between them is different presentation of hash,
//       which in HTML version is a link to repository on Github
//       and in text version is just short hash
pub struct Version {}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{pkg_version} ({hash} {date}{dirty})",
            pkg_version = env!("CARGO_PKG_VERSION"),
            hash = env!("GIT_STATUS_HASH"),
            date = build_time::build_time_local!("%Y-%m-%d %H:%M"),
            dirty = {
                let dirty = env!("GIT_STATUS_DIRTY");
                if dirty == "dirty" {
                    " dirty"
                } else {
                    ""
                }
            }
        )
    }
}

impl maud::Render for Version {
    fn render(&self) -> Markup {
        let pkg_version = env!("CARGO_PKG_VERSION");
        let hash = env!("GIT_STATUS_HASH");
        let full_hash = env!("GIT_STATUS_FULL_HASH");
        let date = build_time::build_time_local!("%Y-%m-%d %H:%M");
        let dirty = {
            let dirty = env!("GIT_STATUS_DIRTY");
            if dirty == "dirty" {
                " dirty"
            } else {
                ""
            }
        };
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
