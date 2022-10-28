use std::{
  collections::HashMap,
  io::{BufRead, Read, Write},
  iter,
  net::{IpAddr, SocketAddr, TcpListener, TcpStream},
  str::FromStr,
  sync::mpsc::{channel, Receiver, Sender},
  thread,
  time::Duration,
};

use bufstream::BufStream;

struct ListenerEvent {
  sender: Sender<(String, TcpStream)>,
  receiver: Receiver<String>,
}

struct MasterEvent {
  sender: Sender<String>,
  receiver: Receiver<(String, TcpStream)>,
}

pub struct Proxy {}

impl Proxy {
  pub fn run(socket: SocketAddr) {
    let (stream_sender, stream_receiver) = channel::<(String, TcpStream)>();
    let (key_sender, key_receiver) = channel::<String>();
    let mut proxy_master = ProxyMaster::new(
      socket,
      ListenerEvent {
        sender: stream_sender,
        receiver: key_receiver,
      },
    );
    let proxy_listener = ProxyListener::new(
      socket,
      MasterEvent {
        sender: key_sender,
        receiver: stream_receiver,
      },
    );
    thread::spawn(move || {
      proxy_master.run();
    });
    proxy_listener.run();
  }
}

pub struct ProxyMaster {
  masters: HashMap<String, TcpStream>,
  listener_socket: SocketAddr,
  listener_event: ListenerEvent,
}

impl ProxyMaster {
  fn new(listener_socket: SocketAddr, listener_event: ListenerEvent) -> Self {
    Self {
      masters: HashMap::new(),
      listener_socket,
      listener_event,
    }
  }

  fn proxy_handle(&mut self, stream: TcpStream) {
    let target_socket = match stream.peer_addr() {
      Ok(socket) => socket,
      Err(_) => {
        println!("Get target socket failed");
        return;
      }
    };
    let mut buf_stream = BufStream::new(stream);
    let mut lines = vec![];
    loop {
      let mut line = String::new();
      if let Err(e) = buf_stream.read_line(&mut line) {
        println!("Read from {} failed: {}", target_socket, e);
        break;
      }
      if line == "\r\n" {
        break;
      }
      lines.push(line);
    }
    if lines.is_empty() {
      println!("Empty request from {}", target_socket);
      return;
    }
    let chunks: Vec<_> = lines[0].split_whitespace().collect();
    if chunks.is_empty() {
      println!("Bad request from {}", target_socket);
      return;
    }
    match chunks[0] {
      "PING" => {
        if let Err(e) = buf_stream.write_all(b"PONG\r\n\r\n").and_then(|_| buf_stream.flush()) {
          println!("Write `PONG` to {} failed: {}", target_socket, e);
        }
      }
      "PROXY" => {
        if let Err(e) = buf_stream
          .write_all(format!("{}\r\n\r\n", self.listener_socket).as_bytes())
          .and_then(|_| buf_stream.flush())
        {
          println!("Write listener socket to {} failed: {}", target_socket, e);
        } else if chunks.len() < 2 {
          println!("Wrong cmd from {}", target_socket);
        } else if self.masters.contains_key(chunks[1]) {
          println!("The key {} exists", chunks[1]);
        } else {
          self
            .masters
            .insert(chunks[1].to_string(), buf_stream.into_inner().unwrap());
        }
      }
      "SEND" | "RECV" => {
        if chunks.len() < 2 {
          println!("Wrong cmd from {}", target_socket);
        } else if !self.masters.contains_key(chunks[1]) {
          println!("The key {} doesn't exist", chunks[1]);
        } else if let Err(e) = self
          .listener_event
          .sender
          .send((chunks[1].to_string(), buf_stream.into_inner().unwrap()))
        {
          println!("Send TCP stream failed: {}", e);
        }
      }
      "END" => {
        if chunks.len() < 2 {
          println!("Wrong cmd from {}", target_socket);
        } else if !self.masters.contains_key(chunks[1]) {
          println!("The key {} doesn't exist", chunks[1]);
        } else {
          self.masters.remove(chunks[1]);
          println!("The key {} removed", chunks[1]);
          println!("Left nodes: {}", self.masters.len());
        }
      }
      _ => {
        println!("Bad cmd from {}", target_socket);
      }
    }
  }

  fn run(&mut self) {
    let addrs = [
      SocketAddr::from(([0, 0, 0, 0], 7070)),
      SocketAddr::from(([0, 0, 0, 0], 7575)),
    ];
    let listener = match TcpListener::bind(&addrs[..]) {
      Ok(listener) => listener,
      Err(e) => {
        println!("Bind failed: {}", e);
        return;
      }
    };

    if let Err(e) = listener.set_nonblocking(true) {
      println!("Set non blocking for listener failed: {}", e);
      return;
    }

    loop {
      if let Ok((stream, _)) = listener.accept() {
        self.proxy_handle(stream);
      }
      if let Ok(key) = self.listener_event.receiver.recv_timeout(Duration::from_millis(100)) {
        if let Some(master) = self.masters.get_mut(&key) {
          if let Err(e) = master.write_all(b"REQUEST\r\n\r\n").and_then(|_| master.flush()) {
            println!(
              "Write to underlying stream ({}) failed: {}",
              master.peer_addr().unwrap(),
              e
            );
            continue;
          }
        } else {
          println!("Unknown key: {}", key);
          continue;
        }
      }
    }
  }

  pub fn get_transport_stream(key: String, master_stream: TcpStream) -> impl iter::Iterator<Item = TcpStream> {
    let master_socket = master_stream.peer_addr().unwrap();
    let mut master_buf_stream = BufStream::new(master_stream);
    iter::from_fn(move || {
      let mut lines = vec![];
      loop {
        let mut line = String::new();
        if let Err(e) = master_buf_stream.read_line(&mut line) {
          println!("Read data from proxy master failed: {}", e);
          return None;
        }
        if line == "\r\n" {
          break;
        }
        lines.push(line);
      }
      if lines.is_empty() {
        println!("Empty data from proxy master");
        return None;
      }
      if lines[0].trim() == "REQUEST" {
        let mut stream = match TcpStream::connect(master_socket) {
          Ok(stream) => stream,
          Err(e) => {
            println!("Connect to proxy master failed: {}", e);
            return None;
          }
        };
        if let Err(e) = stream
          .write_all(format!("SEND {}\r\n\r\n", key).as_bytes())
          .and_then(|_| stream.flush())
        {
          println!("Write to master stream failed: {}", e);
          return None;
        }
        Some(stream)
      } else {
        None
      }
    })
  }

  pub fn end_proxy(key: &str, socket: SocketAddr) {
    if let Err(e) = TcpStream::connect(socket).and_then(|mut stream| {
      stream
        .write_all(format!("END {}\r\n\r\n", key).as_bytes())
        .and_then(|_| stream.flush())
    }) {
      println!("Send END to proxy master failed: {}", e);
    }
  }
}

struct ProxyListener {
  socket: SocketAddr,
  master_event: MasterEvent,
}

impl ProxyListener {
  pub fn new(socket: SocketAddr, master_event: MasterEvent) -> Self {
    Self { socket, master_event }
  }

  pub fn run(&self) {
    let listener = match TcpListener::bind(self.socket) {
      Ok(listener) => listener,
      Err(e) => {
        println!("Bind to {} failed: {}", self.socket, e);
        return;
      }
    };
    for stream in listener.incoming() {
      match stream {
        Ok(stream) => {
          self.proxy_handle(stream);
        }
        Err(e) => {
          println!("Proxy listener get incoming stream failed: {}", e);
          continue;
        }
      }
    }
  }

  fn proxy_handle(&self, stream: TcpStream) {
    let target_socket = match stream.peer_addr() {
      Ok(socket) => socket,
      Err(_) => {
        println!("Get target socket failed");
        return;
      }
    };
    let mut buf_stream = BufStream::new(stream);
    let mut headers = vec![];
    let mut request_method = None;
    let mut content_length = None;
    let mut key = None;
    loop {
      let mut line = String::new();
      if let Err(e) = buf_stream.read_line(&mut line) {
        println!("Read data from target stream failed: {}", e);
        return;
      }
      if line == "\r\n" {
        break;
      }
      if request_method.is_none() {
        let chunks: Vec<_> = line.split_whitespace().collect();
        if chunks.len() < 2 {
          println!("Bad request from {}", target_socket);
          return;
        }
        request_method = Some(chunks[0].to_string());
        key = Some(chunks[1].trim_start_matches('/').to_string());
      }
      if content_length.is_none() && request_method.as_ref().unwrap() == "POST" && line.starts_with("Content-Length:") {
        content_length = match line.trim().split(':').take(2).last() {
          Some(value) => match value.trim().parse::<u64>() {
            Ok(v) => Some(v),
            Err(e) => {
              println!("Parse content length from header failed: {}", e);
              return;
            }
          },
          None => {
            println!("Parse content length from header failed");
            return;
          }
        }
      }
      headers.push(line);
    }
    let request_method = request_method.unwrap();
    let key = key.unwrap();
    let underlying_stream = match self.get_transport_stream(&key) {
      Some(stream) => stream,
      None => {
        println!("Get underlying TCP stream failed");
        return;
      }
    };

    thread::spawn(move || {
      let underlying_socket = match underlying_stream.peer_addr() {
        Ok(socket) => socket,
        Err(_) => {
          println!("Get underlying socket failed");
          return;
        }
      };

      println!("\nProxy: {} <-> master <-> {}", target_socket, underlying_socket);
      let mut underlying_buf_stream = BufStream::new(underlying_stream);
      for header in &headers {
        if let Err(e) = underlying_buf_stream.write_all(header.as_bytes()) {
          println!("Write to underlying stream failed: {}", e);
          return;
        }
      }
      if let Err(e) = underlying_buf_stream
        .write_all(b"\r\n")
        .and_then(|_| underlying_buf_stream.flush())
      {
        println!("Write to underlying stream failed: {}", e);
        return;
      }
      headers.clear();
      loop {
        let mut line = String::new();
        if let Err(e) = underlying_buf_stream.read_line(&mut line) {
          println!("Read data from underlying stream failed: {}", e);
          return;
        }
        if line == "\r\n" {
          break;
        }
        if content_length.is_none() && request_method == "GET" && line.starts_with("Content-Length:") {
          content_length = match line.trim().split(':').take(2).last() {
            Some(value) => match value.trim().parse::<u64>() {
              Ok(v) => Some(v),
              Err(e) => {
                println!("Parse content length from header failed: {}", e);
                return;
              }
            },
            None => {
              println!("Parse content length from header failed");
              return;
            }
          }
        }
        headers.push(line);
      }
      let content_length = content_length.unwrap();
      for header in &headers {
        if let Err(e) = buf_stream.write_all(header.as_bytes()) {
          println!("Write to target stream failed: {}", e);
          return;
        }
      }
      if let Err(e) = buf_stream.write_all(b"\r\n").and_then(|_| buf_stream.flush()) {
        println!("Write to target stream failed: {}", e);
        return;
      }

      let (reader, writer) = if request_method == "GET" {
        (&mut underlying_buf_stream, &mut buf_stream)
      } else {
        (&mut buf_stream, &mut underlying_buf_stream)
      };
      let mut left_size = content_length;
      let mut buf = [0u8; 16 * 1024];
      while left_size > 0 {
        match reader.read(&mut buf) {
          Ok(n) => {
            if let Err(e) = writer.write_all(&buf[0..n]) {
              println!("Write to stream failed: {}", e);
              return;
            }
            left_size -= n as u64;
          }
          Err(_) => {
            println!("Read from stream failed");
            return;
          }
        }
      }
      if let Err(e) = writer.flush() {
        println!("Flush writer failed: {}", e);
        return;
      }
      if request_method == "POST" {
        let mut buf = vec![];
        if let Err(e) = underlying_buf_stream.read_to_end(&mut buf) {
          println!("Read from underlying stream failed: {}", e);
          return;
        }
        if let Err(e) = buf_stream.write_all(&buf).and_then(|_| buf_stream.flush()) {
          println!("Write to target stream failed: {}", e);
          return;
        }
      }
      println!("Proxy: {} <-> master <-> {} done", target_socket, underlying_socket);
    });
  }

  fn get_transport_stream(&self, key: &str) -> Option<TcpStream> {
    match self.master_event.sender.send(key.to_string()) {
      Ok(_) => match self.master_event.receiver.recv() {
        Ok((recv_key, stream)) => {
          if recv_key == key {
            Some(stream)
          } else {
            None
          }
        }
        Err(e) => {
          println!("Recv from master failed: {}", e);
          None
        }
      },
      Err(e) => {
        println!("Send to master failed: {}", e);
        None
      }
    }
  }
}

pub struct ProxyConsumer {
  pub public_socket: SocketAddr,
  pub master_stream: TcpStream,
}

impl ProxyConsumer {
  fn try_get_one(ip: IpAddr, key: &str) -> Option<Self> {
    let addrs = [SocketAddr::from((ip, 7070)), SocketAddr::from((ip, 7575))];
    for addr in &addrs {
      match TcpStream::connect_timeout(addr, Duration::from_millis(200)) {
        Ok(stream) => {
          if let Err(e) = stream.set_read_timeout(Some(Duration::from_millis(200))) {
            println!("Set read timeout for stream failed: {}", e);
            continue;
          }
          let mut buf_stream = BufStream::new(stream);
          if let Err(e) = buf_stream
            .write_all(format!("PROXY {}\r\n\r\n", key).as_bytes())
            .and_then(|_| buf_stream.flush())
          {
            println!("Writer to proxy failed: {}", e);
            continue;
          }
          let mut lines = vec![];
          loop {
            let mut line = String::new();
            if let Err(e) = buf_stream.read_line(&mut line) {
              println!("Read data from proxy master failed: {}", e);
              break;
            }
            if line == "\r\n" {
              break;
            }
            lines.push(line);
          }
          if lines.len() != 1 {
            println!("Wrong response from proxy master");
            continue;
          }
          match SocketAddr::from_str(lines[0].trim()) {
            Ok(socket) => {
              let stream = buf_stream.into_inner().unwrap();
              if let Err(e) = stream.set_read_timeout(None) {
                println!("Set read timeout for stream failed: {}", e);
                continue;
              }
              return Some(Self {
                public_socket: socket,
                master_stream: stream,
              });
            }
            Err(_) => {
              continue;
            }
          }
        }
        Err(_) => continue,
      }
    }
    None
  }

  pub fn try_get(proxy_servers: &[IpAddr], key: &str) -> Option<Self> {
    let addrs = [
      SocketAddr::from(([127, 0, 0, 1], 7070)),
      SocketAddr::from(([127, 0, 0, 1], 7575)),
    ];
    for addr in &addrs {
      match TcpStream::connect_timeout(addr, Duration::from_millis(100)) {
        Ok(stream) => {
          if let Err(e) = stream.set_read_timeout(Some(Duration::from_millis(100))) {
            println!("Set read timeout of stream failed: {}", e);
            continue;
          }
          let mut buf_stream = BufStream::new(stream);
          if let Err(e) = buf_stream.write_all(b"PING\r\n\r\n").and_then(|_| buf_stream.flush()) {
            println!("Write to proxy master failed: {}", e);
            continue;
          }
          let mut lines = vec![];
          loop {
            let mut line = String::new();
            if let Err(e) = buf_stream.read_line(&mut line) {
              println!("Read data from proxy master failed: {}", e);
              break;
            }
            if line == "\r\n" {
              break;
            }
            lines.push(line);
          }
          if lines.len() != 1 {
            println!("Wrong response from proxy master");
            continue;
          }
          if lines[0].trim() == "PONG" {
            return None;
          } else {
            continue;
          }
        }
        Err(_) => continue,
      }
    }

    for ip in proxy_servers {
      if let Some(proxy) = Self::try_get_one(*ip, key) {
        return Some(proxy);
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
    for interface in interfaces {
      let gateway = interface.gateway.as_ref().unwrap().ip_addr;
      if let Some(proxy) = Self::try_get_one(gateway, key) {
        return Some(proxy);
      }
    }
    None
  }
}
