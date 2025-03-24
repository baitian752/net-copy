use std::{
  net::{IpAddr, SocketAddr},
  str::FromStr,
};

use clap::Parser;
use portpicker::pick_unused_port;
use rand::distributions::{Alphanumeric, DistString};

use net_copy::{
  cli::Cli,
  config::{Config, Mode},
  proxy::{Proxy, ProxyConsumer},
  recv::Recv,
  send::Send,
};

fn main() {
  let cli = Cli::parse();

  let mut prompt_save_config = true;
  let mut config = Config::new(&cli);
  if config.host.is_none() {
    let interfaces = default_net::get_interfaces();
    let interfaces: Vec<_> = interfaces
      .iter()
      .filter(|interface| {
        if interface.ipv4.len() == 1 {
          let addr = interface.ipv4[0].addr;
          if addr.is_loopback() || addr.is_unspecified() {
            false
          } else {
            true
          }
        } else {
          false
        }
      })
      .collect();
    let ip_addr = if interfaces.is_empty() {
      println!("Cannot find any valid network interface");
      return;
    } else if interfaces.len() == 1 {
      prompt_save_config = false;
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
        Ok(_) => match input.trim().parse::<usize>() {
          Ok(value) => value,
          Err(e) => {
            println!("Parse input as integer failed: {}", e);
            return;
          }
        },
        Err(e) => {
          println!("Read line failed: {}", e);
          return;
        }
      };
      let ip_addr = match interfaces.get(ip_index) {
        Some(item) => item.ipv4[0].addr,
        None => {
          println!("IP index out of range");
          return;
        }
      };
      ip_addr
    };
    config.host = Some(IpAddr::V4(ip_addr));
  }
  if prompt_save_config {
    config.save();
  }

  let mode = config.mode.unwrap_or(Mode::Normal);
  let key = config
    .key
    .unwrap_or_else(|| Alphanumeric.sample_string(&mut rand::thread_rng(), 6));
  let socket = SocketAddr::from_str(&format!(
    "{}:{}",
    config.host.unwrap(),
    config.port.unwrap_or_else(|| {
      match pick_unused_port() {
        Some(port) => port,
        None => {
          panic!("Pick unused port failed");
        }
      }
    })
  ))
  .unwrap();
  let reserve = config.reserve;
  let proxy_servers = config.proxy.unwrap_or_default();
  let proxy = if config.no_proxy {
    None
  } else {
    ProxyConsumer::try_get(&proxy_servers, &key)
  };

  cli.files.iter().for_each(|file| {
    if !file.exists() {
      panic!("File not found: {}", file.display());
    }
  });

  match mode {
    Mode::Normal => {
      if cli.files.is_empty() {
        Recv::run(&key, socket, reserve, proxy, config.auto_rename);
      } else {
        Send::run(&key, socket, proxy, cli.files);
      }
    }
    Mode::Proxy => {
      if !cli.files.is_empty() {
        println!("WARNING: The proxy mode has activated, files will be ignored");
      }
      Proxy::run(socket);
    }
  }
}
