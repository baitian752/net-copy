use std::{
  alloc,
  net::{IpAddr, SocketAddr},
  str::FromStr,
};

use cap::Cap;
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

#[global_allocator]
static ALLOCATOR: Cap<alloc::System> = Cap::new(alloc::System, usize::max_value());

fn main() {
  // Set the memory limit to 1GiB.
  ALLOCATOR.set_limit(1 * 1024 * 1024 * 1024).unwrap();

  let cli = Cli::parse();

  let mut promt_save_config = true;
  let mut config = Config::new(&cli);
  if config.host.is_none() {
    let interfaces = default_net::get_interfaces();
    let interfaces: Vec<_> = interfaces
      .iter()
      .filter(|interface| {
        interface.if_type == default_net::interface::InterfaceType::Ethernet && !interface.ipv4.is_empty()
      })
      .collect();
    let ip_addr = if interfaces.is_empty() {
      println!("Cannot find any valid network interface");
      return;
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
    ProxyConsumer::try_get(&proxy_servers, key)
  };

  match mode {
    Mode::Normal => {
      if cli.files.is_empty() {
        Recv::run(key.clone(), socket, reserve, proxy);
      } else {
        Send::run(key.clone(), socket, proxy, cli.files);
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
