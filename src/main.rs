use std::{
  env,
  fs::File,
  io::{Read, Write},
  net::{IpAddr, SocketAddr},
  path::PathBuf,
  str::FromStr,
};

use clap::{Parser, ValueEnum};
use network_interface::{NetworkInterface, NetworkInterfaceConfig};
use portpicker::pick_unused_port;
use rand::distributions::{Alphanumeric, DistString};
use serde_derive::{Deserialize, Serialize};

use net_copy::{Mode, NetCopy};

#[derive(Parser)]
#[command(name = "Net Copy", author, version, about, long_about = None)]
struct Cli {
  /// The files to be sent, empty means serve as receiver
  files: Vec<PathBuf>,

  /// The host ip for the server
  #[clap(short = 'l', long, value_parser)]
  host: Option<IpAddr>,

  /// The port for the server
  #[clap(short = 'p', long, value_parser)]
  port: Option<u16>,

  /// The secret key for the server
  #[clap(short = 'k', long, value_parser, value_name = "STRING")]
  key: Option<String>,

  /// Whether reserve the full path of the received file
  #[clap(short = 'r', long, value_parser, action = clap::ArgAction::SetTrue)]
  reserve: Option<bool>,

  /// Proxy for TCP connection
  #[clap(short = 'x', long, value_parser)]
  proxy: Option<SocketAddr>,

  /// Serve mode
  #[clap(short = 'm', long, value_enum)]
  mode: Option<Mode>,
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
  host: Option<IpAddr>,
  port: Option<u16>,
  key: Option<String>,
  reserve: Option<bool>,
  proxy: Option<SocketAddr>,
  mode: Option<Mode>,
}

impl Config {
  fn from_file() -> Self {
    let mut config_file_path = if env::consts::OS == "windows" {
      if let Ok(p) = env::var("APPDATA") {
        PathBuf::from_str(&p)
      } else {
        PathBuf::from_str("C:")
      }
      .unwrap()
      .join("ncp.toml")
    } else {
      PathBuf::from_str("/etc/ncp.toml").unwrap()
    };
    if let Some(home_path) = home::home_dir() {
      let user_config_file_path = home_path.join(".config").join("ncp.toml");
      if user_config_file_path.is_file() {
        config_file_path = user_config_file_path;
      }
    }
    if config_file_path.is_file() {
      let mut config_str = String::new();
      File::open(config_file_path)
        .expect("Open config file failed")
        .read_to_string(&mut config_str)
        .expect("Read config file failed");
      toml::from_str(&config_str).expect("Parse config file failed")
    } else {
      Self::default()
    }
  }

  fn from_env() -> Self {
    Self {
      host: match env::var("NCP_HOST") {
        Ok(x) => Some(IpAddr::from_str(&x).expect("Parse IP from string failed")),
        Err(_) => None,
      },
      port: match env::var("NCP_PORT") {
        Ok(x) => Some(u16::from_str(&x).expect("Parse port from string failed")),
        Err(_) => None,
      },
      key: match env::var("NCP_KEY") {
        Ok(x) => Some(x),
        Err(_) => None,
      },
      reserve: match env::var("NCP_RESERVE") {
        Ok(x) => Some(FromStr::from_str(&x).expect("Parse reserve from string failed")),
        Err(_) => None,
      },
      proxy: match env::var("NCP_PROXY") {
        Ok(x) => Some(SocketAddr::from_str(&x).expect("Parse socket from string failed")),
        Err(_) => None,
      },
      mode: match env::var("NCP_MODE") {
        Ok(x) => Some(Mode::from_str(&x, true).expect("Parse mode from string failed")),
        Err(_) => None,
      },
    }
  }

  fn from_cli(cli: &Cli) -> Self {
    Self {
      host: cli.host,
      port: cli.port,
      key: cli.key.clone(),
      reserve: cli.reserve,
      proxy: cli.proxy,
      mode: cli.mode.clone(),
    }
  }

  fn merge(&mut self, config: &Self) -> &mut Self {
    if self.host.is_none() {
      self.host = config.host;
    }
    if self.port.is_none() {
      self.port = config.port;
    }
    if self.key.is_none() {
      self.key = config.key.clone();
    }
    if self.reserve.is_none() {
      self.reserve = config.reserve;
    }
    if self.proxy.is_none() {
      self.proxy = config.proxy;
    }
    if self.mode.is_none() {
      self.mode = config.mode.clone();
    }
    self
  }

  pub fn new(cli: &Cli) -> Self {
    let mut config = Self::from_cli(cli);
    config.merge(&Self::from_env()).merge(&Self::from_file());
    config
  }

  pub fn save(&self) {
    let config_file_path = if let Some(home_path) = home::home_dir() {
      home_path.join(".config").join("ncp.toml")
    } else {
      PathBuf::from_str("/etc/ncp.toml").unwrap()
    };

    if config_file_path.is_file() {
      return;
    }
    let mut input = String::new();
    println!("Save config? [y/N]");
    match std::io::stdin().read_line(&mut input) {
      Ok(_) => {
        if input.trim().to_uppercase() == "Y" {
          let config_str = format!(
            "\
              host = \"{}\"\n\
              # port = \n\
              # key = \n\
              # reserve = \n\
              # proxy = \n\
              # mode = \n\
              ",
            self.host.unwrap(),
          );
          File::create(&config_file_path)
            .expect("Create config file failed")
            .write_all(config_str.as_bytes())
            .expect("Write to config file failed");
          println!("Config has been written to {}", config_file_path.to_str().unwrap());
        }
      }
      Err(_) => println!("Save config aborted"),
    }
  }
}

fn main() {
  let cli = Cli::parse();

  let mut config = Config::new(&cli);
  if config.host.is_none() {
    let ifas = NetworkInterface::show().expect("List network interface failed");
    let ifas = ifas
      .iter()
      .filter(|ifa| ifa.addr.is_some() && ifa.addr.unwrap().ip().is_ipv4())
      .collect::<Vec<_>>();
    let ip_addr = if ifas.is_empty() {
      panic!("Cannot find any valid network interface");
    } else if ifas.len() == 1 {
      ifas[0].addr.unwrap().ip()
    } else {
      for (i, ifa) in ifas.iter().enumerate() {
        println!("{i}: <{}> {}", ifa.name, ifa.addr.unwrap().ip());
      }
      println!("{} net interfaces have been found, please choose one:", ifas.len());
      let mut input = String::new();
      let ip_index = match std::io::stdin().read_line(&mut input) {
        Ok(_) => input.trim().parse::<usize>().expect("Parse input as integer failed"),
        Err(_) => panic!("Please choose one IP address"),
      };
      let ip_addr = ifas.get(ip_index).expect("IP index out of range").addr.unwrap().ip();
      ip_addr
    };
    config.host = Some(ip_addr);
  }
  config.save();

  let mode = config.mode.unwrap_or(Mode::Normal);
  let key = &config
    .key
    .unwrap_or_else(|| Alphanumeric.sample_string(&mut rand::thread_rng(), 6));
  let socket = SocketAddr::from_str(&format!(
    "{}:{}",
    config.host.unwrap(),
    config.port.unwrap_or_else(|| {
      match pick_unused_port() {
        Some(port) => port,
        None => panic!("Pick unused port failed"),
      }
    })
  ))
  .unwrap();
  let reserve = config.reserve.unwrap_or(false);

  let upload_html = include_bytes!("html/upload.html");
  NetCopy::new(mode, cli.files, key, socket, upload_html, reserve).run();
}
