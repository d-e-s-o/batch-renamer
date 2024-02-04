#![allow(clippy::let_and_return, clippy::let_unit_value)]

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
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
pub fn format_command<C, A, S>(command: C, args: A) -> String
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  concat_command(command, args).to_string_lossy().to_string()
}


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


pub async fn rename(file: &Path, command: &[OsString], dry_run: bool) -> Result<PathBuf> {
  let tmp = tempdir().context("failed to create temporary directory")?;
  let path = canonicalize(file)
    .await
    .with_context(|| format!("failed to canonicalize `{}`", file.display()))?;
  let dir = path
    .parent()
    .with_context(|| format!("`{}` does not contain a parent", path.display()))?;
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

  if !dry_run {
    // Perform the rename on the live data.
    let () = run_in(cmd, cmd_args.iter().chain([&file.to_os_string()]), dir).await?;
  }

  let new_path = dir.join(new.file_name());
  Ok(new_path)
}
