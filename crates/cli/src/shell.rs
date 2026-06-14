use std::fmt;
use std::io::Write;

use anstyle::{AnsiColor, Color, Style};

const GREEN_BOLD: Style = Style::new()
  .fg_color(Some(Color::Ansi(AnsiColor::Green)))
  .bold();
const YELLOW_BOLD: Style = Style::new()
  .fg_color(Some(Color::Ansi(AnsiColor::Yellow)))
  .bold();
const RESET: anstyle::Reset = anstyle::Reset;

pub fn status(label: &str, message: impl fmt::Display) {
  let _ = writeln!(
    anstream::stderr(),
    "{GREEN_BOLD}{label:>12}{RESET} {message}"
  );
}

pub fn warn(message: impl fmt::Display) {
  let label = "warning";
  let _ = writeln!(
    anstream::stderr(),
    "{YELLOW_BOLD}{label:>12}:{RESET} {message}"
  );
}
