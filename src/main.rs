#![allow(clippy::let_and_return, clippy::let_unit_value)]

use std::env::args_os;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::str;

use anyhow::bail;
use anyhow::Context as _;
use anyhow::Error;
use anyhow::Result;

use futures::stream;
use futures::stream::StreamExt as _;

use tokio::task::spawn_blocking;


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
fn format_command<C, A, S>(command: C, args: A) -> String
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S>,
  S: AsRef<OsStr>,
{
  concat_command(command, args).to_string_lossy().to_string()
}


fn evaluate<C, A, S>(output: &Output, command: C, args: A) -> Result<()>
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

/// Run a command with the provided arguments.
fn run<C, A, S>(command: C, args: A) -> Result<()>
where
  C: AsRef<OsStr>,
  A: IntoIterator<Item = S> + Clone,
  S: AsRef<OsStr>,
{
  let _output = run_impl(command, args, Stdio::null())?;
  Ok(())
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
  let (idx, _arg) = args_os()
    .skip(1)
    .enumerate()
    .find(|(_idx, arg)| arg == "--")
    .with_context(|| "Usage: {} <rename-command-and-args> -- <files...")?;
  let cmd = args_os().skip(1).take(idx).collect::<Vec<_>>();
  let files = args_os().skip(1 + idx + 1).collect::<Vec<_>>();

  let mut src_dst = stream::iter(files.into_iter())
    .map(|file| {
      let cmd = cmd.clone();
      spawn_blocking(move || {
        let output = output(
          "print-rename",
          [OsString::from("--dry-run")]
            .iter()
            .chain(&cmd)
            .chain([&file]),
        )?;
        let path = str::from_utf8(&output)?.trim_end().to_string();
        Result::<_, Error>::Ok((PathBuf::from(file), PathBuf::from(path)))
      })
    })
    .buffered(32);

  while let Some(result) = src_dst.next().await {
    let (mut src, dst) = result??;
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
          let _handle = spawn_blocking(move || {
            run(
              "print-rename",
              cmd.iter().chain([src.as_mut_os_string() as _]),
            )
          });
          break
        },
        b"n" | b"N" => break,
        b"q" => return Ok(()),
        _ => {
          println!(
            "Response '{}' not understood",
            &String::from_utf8_lossy(&output)
          )
        },
      }
    }
  }
  Ok(())
}
