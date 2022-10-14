use std::{
  collections::HashMap,
  io::{BufRead, ErrorKind, Read, Write},
  net::{IpAddr, SocketAddr, TcpListener, TcpStream},
  str::FromStr,
  sync::{Arc, Mutex},
  thread::{self, JoinHandle},
  time::{Duration, Instant},
};

use bufstream::BufStream;
use portpicker::pick_unused_port;

#[derive(PartialEq)]
enum RequestMethod {
  Get,
  Post,
}

type Listeners = Arc<Mutex<HashMap<String, (SocketAddr, JoinHandle<()>)>>>;

pub struct Proxy {
  host: IpAddr,
  listeners: Listeners,
}

impl Proxy {
  pub fn new(host: IpAddr) -> Self {
    Self {
      host,
      listeners: Arc::new(Mutex::new(HashMap::new())),
    }
  }

  pub fn run(&mut self) {
    let addrs = [
      SocketAddr::from(([0, 0, 0, 0], 7070)),
      SocketAddr::from(([0, 0, 0, 0], 7575)),
    ];
    let master = TcpListener::bind(&addrs[..]).unwrap();
    master.set_nonblocking(true).unwrap();
    'main_loop: for stream in master.incoming() {
      match stream {
        Ok(stream) => {
          let target_socket = match stream.peer_addr() {
            Ok(socket) => socket,
            Err(_) => {
              println!("Get target socket failed");
              continue 'main_loop;
            }
          };
          let mut buf_stream = BufStream::new(stream);
          let mut line = String::new();
          let mut key: Option<String> = None;
          let mut underlying_socket: Option<SocketAddr> = None;
          loop {
            if buf_stream.read_line(&mut line).is_err() {
              println!("Read from {} failed", target_socket);
              continue 'main_loop;
            }
            if line == "\r\n" {
              break;
            }
            let chunks = line.split_whitespace().collect::<Vec<_>>();
            if chunks.is_empty() {
              println!("Bad request from {}", target_socket);
              continue 'main_loop;
            }
            if chunks[0] == "PROXY" {
              if chunks.len() < 3 {
                println!("Parse request from {} failed", target_socket);
                continue 'main_loop;
              }
              key = Some(chunks[1].to_string());
              match SocketAddr::from_str(chunks[2]) {
                Ok(socket) => underlying_socket = Some(socket),
                Err(_) => {
                  println!("Parsr socker from {} failed", target_socket);
                  continue 'main_loop;
                }
              }
            } else if chunks[0] == "CHECK" {
              if buf_stream
                .write_all(b"OK\r\n\r\n")
                .and_then(|_| buf_stream.flush())
                .is_err()
              {
                println!("Write to {} failed", target_socket);
              }
              continue 'main_loop;
            } else {
              println!("Invalid request method, discarded");
              continue 'main_loop;
            }
            line.clear();
          }
          if key.is_none() {
            println!("Parse request failed from {} failed", target_socket);
            continue 'main_loop;
          }
          let port = match pick_unused_port() {
            Some(port) => port,
            None => {
              println!("Pick unused port failed");
              continue 'main_loop;
            }
          };
          if buf_stream
            .write_all(format!("{}:{}\r\n\r\n", self.host, port).as_bytes())
            .and_then(|_| buf_stream.flush())
            .is_err()
          {
            println!("Writer to {} failed", target_socket);
            continue 'main_loop;
          }
          self.register_target(&key.unwrap(), Some(port), underlying_socket.unwrap());
        }
        Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
          thread::sleep(Duration::from_millis(100));
          continue 'main_loop;
        }
        Err(_) => continue 'main_loop,
      }
    }
  }

  fn register_target(&mut self, key: &str, listen_port: Option<u16>, target_socket: SocketAddr) {
    let mut listeners = self.listeners.lock().unwrap();
    if listeners.contains_key(key) && !listeners.get(key).unwrap().1.is_finished() {
      panic!("The key {} has bounden to {}", key, target_socket);
    }
    let listen_port = listen_port.unwrap_or_else(|| match pick_unused_port() {
      Some(port) => port,
      None => panic!("Pick unused port failed"),
    });
    let listener = match TcpListener::bind(SocketAddr::from_str(&format!("{}:{}", self.host, listen_port)).unwrap()) {
      Ok(listener) => listener,
      Err(_) => {
        panic!("Bind to port {} failed", listen_port);
      }
    };
    if listener.set_nonblocking(true).is_err() {
      panic!("Set nonblocking for proxy stream failed");
    }
    let key_string = key.to_string();
    let listeners_cloned = self.listeners.clone();
    let task = thread::spawn(move || {
      let mut last_active_time = Instant::now();
      let proxy_socket = listener.local_addr().unwrap();
      'main_loop: for stream in listener.incoming() {
        match stream {
          Ok(from_stream) => {
            let from_socket = match from_stream.peer_addr() {
              Ok(socket) => socket,
              Err(_) => {
                println!("Get from socket failed");
                continue 'main_loop;
              }
            };
            println!("\nProxy: {} <-> {} <-> {}", from_socket, proxy_socket, target_socket);
            if from_stream.set_read_timeout(Some(Duration::from_secs(1))).is_err() {
              println!("Set read timeout for {} failed", from_socket);
              continue 'main_loop;
            }
            let mut from_stream = BufStream::new(from_stream);
            let mut target_stream = match TcpStream::connect(target_socket) {
              Ok(stream) => BufStream::new(stream),
              Err(e) => {
                println!("Connect to {} failed", target_socket);
                println!("{:?}", e);
                continue 'main_loop;
              }
            };
            let mut request_line = String::new();
            let mut response_line = String::new();
            if from_stream.read_line(&mut request_line).is_err() {
              println!("Read from {} failed", from_socket);
              continue 'main_loop;
            }
            if target_stream
              .write_all(request_line.as_bytes())
              .and_then(|_| target_stream.flush())
              .is_err()
            {
              println!("Write to {} failed", target_socket);
              continue 'main_loop;
            }
            if target_stream.read_line(&mut response_line).is_err() {
              println!("Read from {} failed", target_socket);
              continue 'main_loop;
            }
            if from_stream
              .write_all(response_line.as_bytes())
              .and_then(|_| from_stream.flush())
              .is_err()
            {
              println!("Write to {} failed", from_socket);
              continue 'main_loop;
            }
            let method = if request_line.starts_with("GET") {
              RequestMethod::Get
            } else if request_line.starts_with("POST") {
              RequestMethod::Post
            } else {
              if from_stream
                .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                .and_then(|_| from_stream.flush())
                .is_err()
              {
                println!("Write to {} failed", from_socket);
              }
              continue 'main_loop;
            };
            println!("Request: {}", request_line.trim());
            println!("Response: {}", response_line.trim());
            let mut header = String::new();
            let mut content_length: Option<u64> = None;
            loop {
              if from_stream.read_line(&mut header).is_err() {
                println!("Read from {} failed", from_socket);
                continue 'main_loop;
              }
              if target_stream.write_all(header.as_bytes()).is_err() {
                println!("Write to {} failed", target_socket);
                continue 'main_loop;
              }
              if header == "\r\n" {
                if target_stream.flush().is_err() {
                  println!("Flush write to {} failed", target_socket);
                  continue 'main_loop;
                }
                break;
              }
              if method == RequestMethod::Post && header.starts_with("Content-Length:") {
                match header.split(':').take(2).last() {
                  Some(s) => match u64::from_str(s.trim()) {
                    Ok(n) => content_length = Some(n),
                    Err(_) => {
                      println!("Parse Content-Length to integet from {} failed", from_socket);
                      continue 'main_loop;
                    }
                  },
                  None => {
                    println!("Parse Content-Length header from {} failed", from_socket);
                    continue 'main_loop;
                  }
                }
              }
              header.clear();
            }
            header.clear();
            loop {
              if target_stream.read_line(&mut header).is_err() {
                println!("Read from {} failed", target_socket);
                continue 'main_loop;
              }
              if from_stream.write_all(header.as_bytes()).is_err() {
                println!("Write to {} failed", from_socket);
                continue 'main_loop;
              }
              if header == "\r\n" {
                if from_stream.flush().is_err() {
                  println!("Flush write to {} failed", from_socket);
                  continue 'main_loop;
                }
                break;
              }
              if header.starts_with("Content-Length:") {
                match header.split(':').take(2).last() {
                  Some(s) => match u64::from_str(s.trim()) {
                    Ok(n) => content_length = Some(n),
                    Err(_) => {
                      println!("Parse Content-Length to integet from {} failed", target_socket);
                      continue 'main_loop;
                    }
                  },
                  None => {
                    println!("Parse Content-Length header from {} failed", target_socket);
                    continue 'main_loop;
                  }
                }
              }
              header.clear();
            }
            if content_length.is_none() {
              println!("Content-Length has not been set, discarded");
              continue 'main_loop;
            }
            let mut buf = [0u8; 16 * 1024];
            let (reader, writer) = match method {
              RequestMethod::Get => (&mut target_stream, &mut from_stream),
              RequestMethod::Post => (&mut from_stream, &mut target_stream),
            };
            let mut left_size = content_length.unwrap();
            while left_size > 0 {
              match reader.read(&mut buf) {
                Ok(n) => {
                  if writer.write_all(&buf[0..n]).is_err() {
                    println!("Write to stream failed");
                    continue 'main_loop;
                  }
                  left_size -= n as u64;
                }
                Err(_) => {
                  println!("Read from stream failed");
                  continue 'main_loop;
                }
              }
            }
            if writer.flush().is_err() {
              println!("Flush writer failed");
              continue 'main_loop;
            }

            if method == RequestMethod::Post {
              let mut buf = vec![];
              if target_stream.read_to_end(&mut buf).is_err() {
                println!("Read from {} failed", target_socket);
                continue 'main_loop;
              }
              if from_stream.write_all(&buf).is_err() {
                println!("Write to {} failed", from_socket);
                continue 'main_loop;
              }
            }
            println!("Proxy: {} <-> {} <-> {} done", from_socket, proxy_socket, target_socket);
            last_active_time = Instant::now();
          }
          Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
            let elapsed = Instant::now() - last_active_time;
            if elapsed.as_secs() > 600 {
              match TcpStream::connect(target_socket) {
                Ok(_) => continue 'main_loop,
                Err(_) => {
                  println!(
                    "The target {} has closed connection, proxy for it will be removed",
                    target_socket
                  );
                  break 'main_loop;
                }
              }
            }
          }
          Err(_) => continue 'main_loop,
        }
      }
      listeners_cloned.lock().unwrap().remove(&key_string);
    });
    listeners.insert(key.to_string(), (target_socket, task));
  }
}
