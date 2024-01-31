#![allow(clippy::let_and_return, clippy::let_unit_value)]

use std::env::args_os;
use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::Context as _;
use anyhow::Result;

use clap::error::ErrorKind;
use clap::Parser;

use batch_rename::rename;


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

  let new_path = rename(&args.file, &args.command, args.dry_run)?;
  println!("{}", new_path.display());
  Ok(())
}
