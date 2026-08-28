// Copyright (C) 2024-2026 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

//! Building blocks for batch renaming of files.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::process::Output;
use std::process::Stdio;

use anyhow::bail;
use anyhow::Context as _;
use anyhow::Result;

use tempfile::tempdir;

use tokio::fs::canonicalize;
use tokio::fs::read_dir;
use tokio::fs::write;
use tokio::process::Command;


/// Concatenate a command and its arguments into a single string.
fn concat_command<C, A, S>(command: C, args: A) -> OsString
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  args
    .into_iter()
    .fold(command.as_ref().to_os_string(), |mut cmd, arg| {
      cmd.push(OsStr::new(" "));
      cmd.push(arg.as_ref());
      cmd
    })
}

/// Format a command with the given list of arguments as a string.
#[doc(hidden)]
pub fn format_command<C, A, S>(command: C, args: A) -> String
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  concat_command(command, args).to_string_lossy().to_string()
}


#[doc(hidden)]
pub fn evaluate<C, A, S>(output: &Output, command: C, args: A) -> Result<()>
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  if !output.status.success() {
    let code = if let Some(code) = output.status.code() {
      format!(" ({code})")
    } else {
      " (terminated by signal)".to_string()
    };

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim_end();
    let stderr = if !stderr.is_empty() {
      format!(": {stderr}")
    } else {
      String::new()
    };

    bail!(
      "`{}` reported non-zero exit-status{code}{stderr}",
      format_command(command, args),
    );
  }
  Ok(())
}


/// Run a command with the provided arguments.
async fn run_in_impl<C, A, S, D>(command: C, args: A, dir: D, stdout: Stdio) -> Result<Output>
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S> + Clone,
  S: AsRef<OsStr>,
  D: AsRef<Path>,
{
  let output = Command::new(command.as_ref())
    .current_dir(dir)
    .stdin(Stdio::null())
    .stdout(stdout)
    .stderr(Stdio::piped())
    .args(args.clone())
    .output()
    .await
    .with_context(|| {
      format!(
        "failed to run `{}`",
        format_command(command.as_ref(), args.clone())
      )
    })?;

  let () = evaluate(&output, command, args)?;
  Ok(output)
}

/// Run a command with the provided arguments.
async fn run_in<C, A, S, D>(command: C, args: A, dir: D) -> Result<()>
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S> + Clone,
  S: AsRef<OsStr>,
  D: AsRef<Path>,
{
  let _output = run_in_impl(command, args, dir, Stdio::null()).await?;
  Ok(())
}


/// Simulate a rename of a file using the provided command.
///
/// The rename is performed in a temporary directory and returned is
/// only the new file name, excluding any path.
pub async fn simulate_rename(path: &Path, command: &[OsString]) -> Result<OsString> {
  let tmp = tempdir().context("failed to create temporary directory")?;
  let path = canonicalize(path)
    .await
    .with_context(|| format!("failed to canonicalize `{}`", path.display()))?;
  let file = path
    .file_name()
    .with_context(|| format!("path `{}` does not have file name", path.display()))?;
  let tmp_file = tmp.path().join(file);
  let () = write(&tmp_file, b"")
    .await
    .with_context(|| format!("failed to create `{}`", tmp_file.display()))?;

  let (cmd, cmd_args) = command.split_first().context("rename command is missing")?;
  // Perform the rename in our temporary directory.
  let () = run_in(
    cmd,
    cmd_args.iter().chain([&file.to_os_string()]),
    tmp.path(),
  )
  .await?;

  let new = read_dir(tmp.path())
    .await
    .with_context(|| {
      format!(
        "failed to read contents of directory `{}`",
        tmp.path().display()
      )
    })?
    .next_entry()
    .await
    .with_context(|| {
      format!(
        "no file found in `{}`; did the rename operation delete instead?",
        tmp.path().display()
      )
    })?
    .with_context(|| format!("failed to read first file of `{}`", tmp.path().display()))?;

  Ok(new.file_name())
}
