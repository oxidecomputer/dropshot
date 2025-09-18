// Copyright 2025 Oxide Computer Company

//! Helpers for accessing data stored in git

use anyhow::{bail, Context};
use camino::{Utf8Path, Utf8PathBuf};
use std::process::Command;

/// Newtype String wrapper identifying a Git revision
///
/// This could be a commit, branch name, tag name, etc.  This type does not
/// validate the contents.
#[derive(Clone, Ord, PartialOrd, Eq, PartialEq)]
pub struct GitRevision(String);
NewtypeDebug! { () pub struct GitRevision(String); }
NewtypeDeref! { () pub struct GitRevision(String); }
NewtypeDerefMut! { () pub struct GitRevision(String); }
NewtypeDisplay! { () pub struct GitRevision(String); }
NewtypeFrom! { () pub struct GitRevision(String); }

/// Given a revision, return its merge base with HEAD
pub fn git_merge_base_head(
    revision: &GitRevision,
) -> anyhow::Result<GitRevision> {
    let mut cmd = git_start();
    cmd.arg("merge-base").arg("--all").arg("HEAD").arg(revision.as_str());
    let label = cmd_label(&cmd);
    let stdout = do_run(&mut cmd)?;
    let stdout = stdout.trim();
    if stdout.contains(" ") || stdout.contains("\n") {
        bail!(
            "unexpected output from {} (contains whitespace -- \
             multiple merge bases?)",
            label
        );
    }
    Ok(GitRevision::from(stdout.to_owned()))
}

/// List files recursively under some path `path` in Git revision `revision`.
pub fn git_ls_tree(
    revision: &GitRevision,
    directory: &Utf8Path,
) -> anyhow::Result<Vec<Utf8PathBuf>> {
    let mut cmd = git_start();
    cmd.arg("ls-tree")
        .arg("-r")
        .arg("-z")
        .arg("--name-only")
        .arg("--full-tree")
        .arg(revision.as_str())
        .arg(&directory);
    let label = cmd_label(&cmd);
    let stdout = do_run(&mut cmd)?;
    stdout
        .trim()
        .split("\0")
        .filter(|s| !s.is_empty())
        .map(|path| {
            let found_path = Utf8PathBuf::from(path);
            let Ok(relative) = found_path.strip_prefix(directory) else {
                bail!(
                "git ls-tree unexpectedly returned a path that did not start \
                 with {:?}: {:?} (cmd: {})",
                directory,
                found_path,
                label,
            );
            };
            Ok(relative.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()
}

/// Returns the contents of the file at the given path `path` in Git revision
/// `revision`.
pub fn git_show_file(
    revision: &GitRevision,
    path: &Utf8Path,
) -> anyhow::Result<Vec<u8>> {
    let mut cmd = git_start();
    cmd.arg("cat-file").arg("blob").arg(format!("{}:{}", revision, path));
    let stdout = do_run(&mut cmd)?;
    Ok(stdout.into_bytes())
}

/// Begin assembling an invocation of git(1)
fn git_start() -> Command {
    let git = std::env::var("GIT").ok().unwrap_or_else(|| String::from("git"));
    Command::new(&git)
}

/// Runs an assembled git(1) command, returning stdout on success and an error
/// including the exit status and stderr contents on failure.
fn do_run(cmd: &mut Command) -> anyhow::Result<String> {
    let label = cmd_label(cmd);
    let output = cmd.output().with_context(|| format!("invoking {:?}", cmd))?;
    let status = output.status;
    let stdout = output.stdout;
    let stderr = output.stderr;
    if status.success() {
        if let Ok(stdout) = String::from_utf8(stdout) {
            return Ok(stdout);
        } else {
            bail!("command succeeded, but output was not UTF-8: {}:\n", label);
        }
    }

    bail!(
        "command failed: {}: {}\n\
        stderr:\n\
        -----\n\
        {}\n\
        -----\n",
        label,
        status,
        String::from_utf8_lossy(&stderr)
    );
}

/// Returns a string describing an assembled command (for debugging and error
/// reporting)
fn cmd_label(cmd: &Command) -> String {
    format!(
        "{:?} {}",
        cmd.get_program(),
        cmd.get_args()
            .map(|a| format!("{:?}", a))
            .collect::<Vec<_>>()
            .join(" ")
    )
}
