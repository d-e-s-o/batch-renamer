// Copyright (C) 2024 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

#![allow(clippy::let_and_return, clippy::let_unit_value)]

use std::env::args_os;
use std::ffi::OsString;
use std::io::stdout;
use std::io::Write as _;
use std::os::unix::ffi::OsStrExt as _;
use std::path::PathBuf;

use anyhow::Result;

use clap::error::ErrorKind;
use clap::Parser;

use batch_renamer::rename;


#[derive(Debug, Parser)]
#[clap(name = "print-rename", version = env!("VERSION"))]
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

  let new_path = rename(&args.file, &args.command, args.dry_run).await?;
  let () = stdout().write_all(new_path.as_os_str().as_bytes())?;
  Ok(())
}
