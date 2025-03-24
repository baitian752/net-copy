use std::{
  env,
  fs::{create_dir_all, File},
  io::{Read, Write},
  net::IpAddr,
  path::PathBuf,
  str::FromStr,
};

use clap::ValueEnum;
use serde_derive::{Deserialize, Serialize};

use crate::cli::Cli;

#[derive(ValueEnum, Clone, Debug, Serialize, Deserialize)]
pub enum Mode {
  Normal,
  Proxy,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
  pub host: Option<IpAddr>,
  pub port: Option<u16>,
  pub key: Option<String>,
  pub reserve: bool,
  pub proxy: Option<Vec<IpAddr>>,
  pub no_proxy: bool,
  pub mode: Option<Mode>,
  pub auto_rename: bool,
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
      auto_rename: match env::var("NCP_AUTO_RENAME") {
        Ok(x) => FromStr::from_str(&x).unwrap(),
        Err(_) => false,
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
      auto_rename: cli.auto_rename,
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
    if !self.auto_rename {
      self.auto_rename = config.auto_rename;
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
          if let Some(parent) = config_file_path.parent() {
            if !parent.is_dir() {
              if let Err(e) = create_dir_all(parent) {
                println!("Create directoty {:?} for config file failed: {}", parent, e);
                return;
              }
            }
          }
          let config_str = format!(
            "\
              host = \"{}\"\n\
              # port = \n\
              # key = \n\
              reserve = false\n\
              proxy = []\n\
              no_proxy = false\n\
              # mode = \"normal\"\n\
              auto_rename = false\n\
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
