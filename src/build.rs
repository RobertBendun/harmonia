//! Prepare environment variables for build
use std::ffi::OsStr;
use std::process::Command;

/// Get stdout of command
fn get_output<S, I>(program: S, args: I) -> Result<String, std::io::Error>
where
    S: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
{
    let finished = Command::new(program).args(args).output()?;

    assert!(finished.status.success());
    Ok(String::from_utf8(finished.stdout).unwrap())
}

fn main() {
    let git_hash = get_output("git", ["rev-parse", "--short", "HEAD"]).unwrap();
    let git_hash_full = get_output("git", ["rev-parse", "HEAD"]).unwrap();
    let clean = get_output("git", ["status", "--untracked-files=no", "--porcelain"])
        .unwrap()
        .is_empty();

    println!("cargo:rustc-env=GIT_STATUS_FULL_HASH={git_hash_full}");
    println!("cargo:rustc-env=GIT_STATUS_HASH={git_hash}");
    println!(
        "cargo:rustc-env=GIT_STATUS_DIRTY={dirty}",
        dirty = if clean { "" } else { "dirty" }
    );
}
