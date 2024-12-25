// Copyright (C) 2024 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(clippy::let_and_return, clippy::let_unit_value)]

use std::env::args_os;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::future::ready;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use anyhow::Context as _;
use anyhow::Error;
use anyhow::Result;

use batch_renamer::evaluate;
use batch_renamer::format_command;
use batch_renamer::rename;

use clap::error::ErrorKind;
use clap::Parser;

use futures::stream;
use futures::stream::FuturesUnordered;
use futures::stream::StreamExt as _;
use futures::TryStreamExt as _;

use tokio::spawn;
use tokio::task::spawn_blocking;


#[derive(Debug, Parser)]
#[clap(version = env!("VERSION"))]
struct Args {
  /// The command (and arguments) to use for renaming the file(s).
  #[clap(required = true)]
  command: Vec<OsString>,
  /// The files to rename.
  #[clap(last = true)]
  files: Vec<PathBuf>,
}


/// Run a command with the provided arguments.
fn run_impl<C, A, S>(command: C, args: A, stdout: Stdio) -> Result<Output>
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S> + Clone,
  S: AsRef<OsStr>,
{
  let output = Command::new(command.as_ref())
    .stdin(Stdio::inherit())
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

/// Run a command and capture its output.
fn output<C, A, S>(command: C, args: A) -> Result<Vec<u8>>
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S> + Clone,
  S: AsRef<OsStr>,
{
  let output = run_impl(command, args, Stdio::piped())?;
  Ok(output.stdout)
}


#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
  let args = match Args::try_parse_from(args_os()) {
    Ok(args) => args,
    Err(err) => match err.kind() {
      ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
        print!("{}", err);
        return Ok(())
      },
      _ => return Err(err.into()),
    },
  };

  let Args {
    command: cmd,
    files,
  } = args;

  let mut src_dst = stream::iter(files.into_iter())
    .map(|file| async {
      let path = rename(Path::new(&file), &cmd, true).await?;
      Result::<_, Error>::Ok((file, path))
    })
    .buffered(32);

  let renames = FuturesUnordered::new();

  'outer: while let Some(result) = src_dst.next().await {
    let (src, dst) = result?;
    let src_file = src
      .file_name()
      .with_context(|| format!("path `{}` does not have file name", src.display()))?;
    let src_file = Path::new(src_file);
    let dst_file = dst
      .file_name()
      .with_context(|| format!("path `{}` does not have file name", dst.display()))?;
    let dst_file = Path::new(dst_file);

    if src_file == dst_file {
      continue
    }

    loop {
      println!(
        "Would rename:\n\x1b[1;34m{}\x1b[0m\nto\n\x1b[1;34m{}\x1b[0m\nAccept? (Y/n/q)\x1b[0m",
        src_file.display(),
        dst_file.display()
      );

      let output =
        spawn_blocking(|| output("bash", ["-c", "read -s -n 1 value && echo -n \"${value}\""]))
          .await??;

      match output.as_slice() {
        b"" | b"y" | b"Y" => {
          let cmd = cmd.clone();
          let handle = spawn(async move {
            let _path = rename(&src, &cmd, false).await?;
            Result::<_, Error>::Ok(())
          });
          let () = renames.push(handle);
          break
        },
        b"n" | b"N" => break,
        b"q" => break 'outer,
        _ => {
          println!(
            "Response '{}' not understood",
            &String::from_utf8_lossy(&output)
          )
        },
      }
    }
  }

  // Convert `JoinError` into anyhow's Error and then flatten, before
  // draining all tasks.
  let () = renames
    .map_err(Error::from)
    .and_then(ready)
    .try_for_each_concurrent(Some(64), |()| ready(Ok(())))
    .await?;
  Ok(())
}
