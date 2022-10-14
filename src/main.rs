use std::{
  env,
  fs::File,
  io::{Read, Write, BufRead},
  net::{IpAddr, SocketAddr, TcpStream},
  path::PathBuf,
  str::FromStr, time::Duration,
};

use bufstream::BufStream;
use clap::{Parser, ValueEnum};
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
  #[clap(short = 'r', long, value_parser)]
  reserve: bool,

  /// Proxy for TCP connection
  #[clap(short = 'x', long, value_parser, action = clap::ArgAction::Append)]
  proxy: Option<Vec<IpAddr>>,

  /// Disable automatically check proxy from gateway
  #[clap(short = 'X', long, value_parser)]
  no_proxy: bool,

  /// Serve mode
  #[clap(short = 'm', long, value_enum)]
  mode: Option<Mode>,
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
  host: Option<IpAddr>,
  port: Option<u16>,
  key: Option<String>,
  reserve: bool,
  proxy: Option<Vec<IpAddr>>,
  no_proxy: bool,
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
        Ok(x) => Some(IpAddr::from_str(&x).unwrap()),
        Err(_) => None,
      },
      port: match env::var("NCP_PORT") {
        Ok(x) => Some(u16::from_str(&x).unwrap()),
        Err(_) => None,
      },
      key: match env::var("NCP_KEY") {
        Ok(x) => Some(x),
        Err(_) => None,
      },
      reserve: match env::var("NCP_RESERVE") {
        Ok(x) => FromStr::from_str(&x).unwrap(),
        Err(_) => false,
      },
      proxy: match env::var("NCP_PROXY") {
        Ok(x) => Some(x.split(':').map(|x| IpAddr::from_str(x).unwrap()).collect::<Vec<_>>()),
        Err(_) => None,
      },
      no_proxy: match env::var("NCP_NO_PROXY") {
        Ok(x) => FromStr::from_str(&x).unwrap(),
        Err(_) => false,
      },
      mode: match env::var("NCP_MODE") {
        Ok(x) => Some(Mode::from_str(&x, true).unwrap()),
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
      proxy: cli.proxy.clone(),
      no_proxy: cli.no_proxy,
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
    if !self.reserve {
      self.reserve = config.reserve;
    }
    if self.proxy.is_none() {
      self.proxy = config.proxy.clone();
    }
    if !self.no_proxy {
      self.no_proxy = config.no_proxy;
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
              reserve = false\n\
              # proxy = \n\
              no_proxy = false\n\
              # mode = \"normal\"\n\
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

fn check_proxy(ip: IpAddr, key: &str, socket: &SocketAddr) -> Option<SocketAddr> {
  let addrs = [SocketAddr::from((ip, 7070)), SocketAddr::from((ip, 7575))];
  for addr in &addrs {
    match TcpStream::connect_timeout(addr, Duration::from_millis(100)) {
      Ok(stream) => {
        if stream.set_read_timeout(Some(Duration::from_millis(100))).is_err() {
          println!("Set read timeout for stream failed");
          continue;
        }
        let mut buf_stream = BufStream::new(stream);
        if buf_stream
          .write_all(format!("PROXY {} {}\r\n\r\n", key, socket).as_bytes())
          .and_then(|_| buf_stream.flush())
          .is_err()
        {
          println!("Writer to proxy failed");
          continue;
        }
        let mut buf = String::new();
        if buf_stream.read_line(&mut buf).is_err() {
          println!("Read from proxy failed");
          continue;
        }
        match SocketAddr::from_str(buf.trim()) {
          Ok(socket) => {
            return Some(socket);
          }
          Err(_) => {
            println!("Parse socket from {} failed", addr);
            continue;
          }
        }
      }
      Err(_) => continue,
    }
  }
  None
}

fn get_proxy(proxy_servers: &[IpAddr], key: &str, socket: SocketAddr) -> Option<SocketAddr> {
  let addrs = [
    SocketAddr::from(([127, 0, 0, 1], 7070)),
    SocketAddr::from(([127, 0, 0, 1], 7575)),
  ];
  for addr in &addrs {
    match TcpStream::connect_timeout(addr, Duration::from_millis(100)) {
      Ok(stream) => {
        stream.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let mut buf_stream = BufStream::new(stream);
        buf_stream.write_all(b"CHECK\r\n\r\n").unwrap();
        buf_stream.flush().unwrap();
        let mut buf = String::new();
        match buf_stream.read_line(&mut buf) {
          Ok(_) => {
            if buf.trim() == "OK" {
              return None;
            }
            continue;
          }
          Err(_) => continue,
        }
      }
      Err(_) => continue,
    }
  }
  let interfaces = default_net::get_interfaces();
  let interfaces = interfaces
    .iter()
    .filter(|interface| {
      interface.if_type == default_net::interface::InterfaceType::Ethernet
        && !interface.ipv4.is_empty()
        && interface.gateway.is_some()
    })
    .collect::<Vec<_>>();
  for ip in proxy_servers {
    if let Some(proxy) = check_proxy(*ip, key, &socket) {
      return Some(proxy);
    }
  }
  for interface in interfaces {
    let gateway = interface.gateway.as_ref().unwrap().ip_addr;
    if let Some(proxy) = check_proxy(gateway, key, &socket) {
      return Some(proxy);
    }
  }
  None
}

fn main() {
  let cli = Cli::parse();

  let mut promt_save_config = true;
  let mut config = Config::new(&cli);
  if config.host.is_none() {
    let interfaces = default_net::get_interfaces();
    let interfaces = interfaces
      .iter()
      .filter(|interface| {
        interface.if_type == default_net::interface::InterfaceType::Ethernet && !interface.ipv4.is_empty()
      })
      .collect::<Vec<_>>();
    let ip_addr = if interfaces.is_empty() {
      panic!("Cannot find any valid network interface");
    } else if interfaces.len() == 1 {
      promt_save_config = false;
      interfaces[0].ipv4[0].addr
    } else {
      for (i, interface) in interfaces.iter().enumerate() {
        println!("{i}: <{}> {}", interface.name, interface.ipv4[0].addr);
      }
      println!(
        "{} net interfaces have been found, please choose one:",
        interfaces.len()
      );
      let mut input = String::new();
      let ip_index = match std::io::stdin().read_line(&mut input) {
        Ok(_) => input.trim().parse::<usize>().expect("Parse input as integer failed"),
        Err(_) => panic!("Please choose one IP address"),
      };
      let ip_addr = interfaces.get(ip_index).expect("IP index out of range").ipv4[0].addr;
      ip_addr
    };
    config.host = Some(IpAddr::V4(ip_addr));
  }
  if promt_save_config {
    config.save();
  }

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
  let reserve = config.reserve;
  let proxy_servers = config.proxy.unwrap_or_default();
  let proxy = if config.no_proxy {
    None
  } else {
    get_proxy(&proxy_servers, key, socket)
  };

  let upload_html = include_bytes!("html/upload.html");
  NetCopy::new(mode, cli.files, key, socket, upload_html, reserve, proxy).run();
}
