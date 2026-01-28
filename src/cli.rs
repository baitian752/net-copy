use std::{net::IpAddr, path::PathBuf};

use clap::Parser;

use crate::config::Mode;

#[derive(Parser)]
#[command(name = "Net Copy", author, version, about, long_about = None)]
pub struct Cli {
  /// The files to be sent, empty means serve as receiver
  pub files: Vec<PathBuf>,

  /// The host ip for the server
  #[clap(short = 'l', long, value_parser)]
  pub host: Option<IpAddr>,

  /// The port for the server
  #[clap(short = 'p', long, value_parser)]
  pub port: Option<u16>,

  /// The secret key for the server
  #[clap(short = 'k', long, value_parser, value_name = "STRING")]
  pub key: Option<String>,

  /// Whether reserve the full path of the received file
  #[clap(short = 'r', long, value_parser)]
  pub reserve: bool,

  /// Proxy for TCP connection
  #[clap(short = 'x', long, value_parser, action = clap::ArgAction::Append)]
  pub proxy: Option<Vec<IpAddr>>,

  /// Disable automatically check proxy from gateway
  #[clap(short = 'X', long, value_parser)]
  pub no_proxy: bool,

  /// Serve mode
  #[clap(short = 'm', long, value_enum)]
  pub mode: Option<Mode>,

  /// Auto rename file if exist
  #[clap(short = 'a', long, value_parser)]
  pub auto_rename: bool,

  /// Whether show save config prompt
  #[clap(short = 's', long, value_parser)]
  pub prompt_save_config: bool,
}
