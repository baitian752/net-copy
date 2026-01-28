use std::{
  io::{self, Write},
  net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6},
};

use clap::Parser;
use portpicker::pick_unused_port;
use rand::distr::{Alphanumeric, SampleString};

use net_copy::{
  cli::Cli,
  config::{Config, Mode},
  proxy::{Proxy, ProxyConsumer},
  recv::Recv,
  send::Send,
};

fn main() {
  let cli = Cli::parse();

  let mut config = Config::new(&cli);

  let mut ip_list: Vec<IpAddr> = vec![];
  let interfaces = default_net::get_interfaces();
  for interface in &interfaces {
    if interface.ipv4.len() > 0 {
      for net in &interface.ipv4 {
        if net.addr.is_loopback() || net.addr.is_broadcast() {
          continue;
        }
        ip_list.push(IpAddr::V4(net.addr));
        println!(
          "{}: <{}> {}",
          ip_list.len(),
          interface.friendly_name.as_ref().unwrap_or(&interface.name),
          net.addr
        );
      }
    }
  }
  for interface in &interfaces {
    if interface.ipv6.len() > 0 {
      for net in &interface.ipv6 {
        if net.addr.is_loopback() {
          continue;
        }
        ip_list.push(IpAddr::V6(net.addr));
        println!(
          "{}: <{}> [{}]",
          ip_list.len(),
          interface.friendly_name.as_ref().unwrap_or(&interface.name),
          net.addr
        );
      }
    }
  }

  if ip_list.is_empty() {
    println!("Cannot find any valid network interface");
    return;
  }

  print!("Please choose one in 1..{}: ", ip_list.len());
  io::stdout().flush().unwrap();
  let mut input = String::new();
  let ip_index = match std::io::stdin().read_line(&mut input) {
    Ok(_) => match input.trim().parse::<usize>() {
      Ok(value) => {
        if value < 1 || value > ip_list.len() {
          println!("Index range is 1..{}", ip_list.len());
          return;
        } else {
          value - 1
        }
      }
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

  config.host = Some(ip_list[ip_index]);

  if config.prompt_save_config {
    config.save();
  }

  let mode = config.mode.unwrap_or(Mode::Normal);
  let key = config
    .key
    .unwrap_or_else(|| Alphanumeric.sample_string(&mut rand::rng(), 6));
  let port = config.port.unwrap_or_else(|| match pick_unused_port() {
    Some(port) => port,
    None => {
      panic!("Pick unused port failed");
    }
  });
  let socket: SocketAddr = match config.host.unwrap() {
    IpAddr::V4(addr) => SocketAddr::V4(SocketAddrV4::new(addr, port)),
    IpAddr::V6(addr) => SocketAddr::V6(SocketAddrV6::new(addr, port, 0, 0)),
  };
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
