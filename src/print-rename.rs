#![allow(clippy::let_and_return, clippy::let_unit_value)]

use std::env::args_os;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs::canonicalize;
use std::fs::read_dir;
use std::fs::write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use anyhow::Context as _;
use anyhow::Result;

use batch_rename::evaluate;
use batch_rename::format_command;

use clap::error::ErrorKind;
use clap::Parser;

use tempfile::tempdir;


#[derive(Debug, Parser)]
struct Args {
  /// Do not actually perform the rename.
  #[clap(short = 'n', long = "dry-run")]
  dry_run: bool,
  /// The command (and arguments) to use for renaming the file.
  #[clap(required = true)]
  command: Vec<OsString>,
  /// The file to rename.
  file: PathBuf,
}


/// Run a command with the provided arguments.
fn run_impl<C, A, S, D>(command: C, args: A, dir: D, stdout: Stdio) -> Result<Output>
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
fn run<C, A, S, D>(command: C, args: A, dir: D) -> Result<()>
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S> + Clone,
  S: AsRef<OsStr>,
  D: AsRef<Path>,
{
  let _output = run_impl(command, args, dir, Stdio::null())?;
  Ok(())
}


fn main() -> Result<()> {
  let args = match Args::try_parse_from(args_os()) {
    Ok(args) => args,
    Err(err) => match err.kind() {
      ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
        print!("{}", err);
        return Ok(())
      },
      _ => return Err(err).context("failed to parse program arguments"),
    },
  };

  let tmp = tempdir().context("failed to create temporary directory")?;
  let path = canonicalize(&args.file)
    .with_context(|| format!("failed to canonicalize `{}`", args.file.display()))?;
  let dir = path
    .parent()
    .with_context(|| format!("`{}` does not contain a parent", path.display()))?;
  let file = path
    .file_name()
    .with_context(|| format!("path `{}` does not have file name", path.display()))?;
  let tmp_file = tmp.path().join(file);
  let () =
    write(&tmp_file, b"").with_context(|| format!("failed to create `{}`", tmp_file.display()))?;

  // SANITY: `clap` ensures that there is always at least one string
  //         present.
  let (cmd, cmd_args) = args.command.split_first().unwrap();
  // Perform the rename in our temporary directory.
  let () = run(
    cmd,
    cmd_args.iter().chain([&file.to_os_string()]),
    tmp.path(),
  )?;

  let new = read_dir(tmp.path())
    .with_context(|| {
      format!(
        "failed to read contents of directory `{}`",
        tmp.path().display()
      )
    })?
    .next()
    .with_context(|| {
      format!(
        "no file found in `{}`; did the rename operation delete instead?",
        tmp.path().display()
      )
    })?
    .with_context(|| format!("failed to read first file of `{}`", tmp.path().display()))?;

  if !args.dry_run {
    // Perform the rename on the live data.
    let () = run(cmd, cmd_args.iter().chain([&file.to_os_string()]), dir)?;
  }

  println!("{}", dir.join(new.file_name()).display());
  Ok(())
}
