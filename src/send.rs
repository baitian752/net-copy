use std::{
  fs::{self, File},
  io::{BufRead, BufReader, Read, Write},
  net::{SocketAddr, TcpListener, TcpStream},
  path::{Path, PathBuf},
  process,
  sync::Mutex,
  thread,
};

use bufstream::BufStream;
use mime_guess;
use tar::Builder;

use crate::proxy::ProxyConsumer;

static FILE_PATHS: Mutex<Vec<PathBuf>> = Mutex::new(vec![]);

pub struct Send {
  key: String,
  socket: SocketAddr,
  proxy: Option<ProxyConsumer>,
}

impl Send {
  pub fn new(key: String, socket: SocketAddr, proxy: Option<ProxyConsumer>, file_paths: Vec<PathBuf>) -> Self {
    FILE_PATHS.lock().unwrap().clear();
    for file_path in file_paths {
      FILE_PATHS.lock().unwrap().push(file_path);
    }
    Self { key, socket, proxy }
  }

  pub fn run(&self) {
    self.send();
  }

  fn is_archive() -> bool {
    let file_paths = &FILE_PATHS.lock().unwrap();
    file_paths.len() > 1 || !file_paths[0].is_file()
  }

  fn get_tar_path(key: &str) -> PathBuf {
    PathBuf::from(format!("{}.tar", key))
  }

  fn tar(tar_path: &Path) -> bool {
    let mut tar = match File::create(tar_path) {
      Ok(file) => Builder::new(file),
      Err(e) => {
        println!("Create tar file {:?} failed: {}", tar_path, e);
        return false;
      }
    };
    let file_paths = FILE_PATHS.lock().unwrap();
    for file_path in file_paths.iter() {
      if if file_path.is_dir() {
        tar.append_dir_all(file_path, file_path)
      } else {
        tar.append_path(file_path)
      }
      .is_err()
      {
        println!("Append file to tar failed");
        return false;
      }
    }
    if tar.finish().is_err() {
      println!("Write tar file failed");
      false
    } else {
      true
    }
  }

  fn handle_send(
    stream: TcpStream, key: String, file_path: PathBuf, file_name: String, is_archive: bool, mime_type: String,
  ) {
    if is_archive && !file_path.is_file() && !Self::tar(&file_path) {
      println!("Archive sending files failed");
      return;
    }
    let peer_addr = match stream.peer_addr() {
      Ok(peer_addr) => peer_addr,
      Err(e) => {
        println!("Get peer socket failed: {}", e);
        return;
      }
    };
    let mut buf_stream = BufStream::new(stream);
    let mut headers = vec![];
    loop {
      let mut line = String::new();
      if let Err(e) = buf_stream.read_line(&mut line) {
        println!("Read line from buffer failed: {}", e);
        return;
      }
      if line == "\r\n" {
        break;
      }
      headers.push(line);
    }
    if headers.is_empty() {
      println!("Empty request headers from {}", peer_addr);
      return;
    }
    if !headers[0].trim().starts_with(&format!("GET /{} HTTP/", key)) {
      println!("Bad Request from {}: {}", peer_addr, headers[0].trim());
      if let Err(e) = buf_stream
        .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
        .and_then(|_| buf_stream.flush())
      {
        println!("Write response header failed: {}", e);
        return;
      }
      return;
    }

    println!("\nSending {:?} to {}", file_path, peer_addr);
    let file = File::open(&file_path).expect("Open sending file failed");
    let mut file_reader = BufReader::new(&file);
    let file_size = file.metadata().expect("Get file size metadata failed").len();

    if let Err(e) = buf_stream
      .write_all(
        format!(
          "HTTP/1.1 200 OK\r\n\
          Content-Length: {file_size}\r\n\
          Content-Type: {mime_type}\r\n\
          Content-Disposition: attachment; filename=\"{file_name}\"\r\n\
          \r\n"
        )
        .as_bytes(),
      )
      .and_then(|_| buf_stream.flush())
    {
      println!("Write response header failed: {}", e);
      return;
    }

    let mut buf = vec![0u8; 16 * 1024];
    let mut left_size = file_size;
    while left_size > 0 {
      match file_reader.read(&mut buf) {
        Ok(n) => {
          if let Err(e) = buf_stream.write_all(&buf[0..n]) {
            println!("Write response content failed: {}", e);
            return;
          }
          left_size -= n as u64;
        }
        Err(e) => {
          println!("Read sending file failed: {}", e);
          return;
        }
      }
    }
    if let Err(e) = buf_stream.flush() {
      println!("Flush stream failed: {}", e);
      return;
    }

    let mut buf = vec![];
    if let Err(e) = buf_stream.read_to_end(&mut buf) {
      println!("Read data from stream failed: {}", e);
      return;
    }
    println!("Send {:?} to {} done", file_path, peer_addr);
  }

  fn send(&self) {
    let is_archive = Self::is_archive();
    let file_path = if is_archive {
      let file_path = Self::get_tar_path(&self.key);
      if !file_path.is_file() {
        Self::tar(&file_path);
      }
      file_path
    } else {
      FILE_PATHS.lock().unwrap()[0].clone()
    };
    let file_name = file_path.file_name().unwrap().to_str().unwrap().to_string();
    let mime_type = mime_guess::from_path(&file_path).first_or_octet_stream().to_string();

    let tar_path = file_path.clone();
    if let Err(e) = ctrlc::set_handler(move || {
      if is_archive && tar_path.is_file() {
        fs::remove_file(&tar_path).expect("Remove tar file failed");
      }
      process::exit(0);
    }) {
      println!("Set Ctrl-C handler failed: {}", e);
      return;
    }

    let pub_addr = if let Some(proxy) = &self.proxy {
      proxy.public_socket
    } else {
      self.socket
    };
    println!();
    if is_archive {
      println!("cURL: curl http://{}/{} | tar xvf -", pub_addr, self.key);
      println!("Wget: wget -O- http://{}/{} | tar xvf -", pub_addr, self.key);
      println!("PowerShell: cmd /C 'curl http://{}/{} | tar xvf -'", pub_addr, self.key);
    } else {
      println!("cURL: curl -o {} http://{}/{}", file_name, pub_addr, self.key);
      println!("Wget: wget -O {} http://{}/{}", file_name, pub_addr, self.key);
      println!("PowerShell: iwr -O {} http://{}/{}", file_name, pub_addr, self.key);
    }
    println!("Browser: http://{}/{}", pub_addr, self.key);

    if let Some(proxy) = &self.proxy {
      let mut master_buf_stream = BufStream::new(&proxy.master_stream);
      loop {
        let mut lines = vec![];
        loop {
          let mut line = String::new();
          if let Err(e) = master_buf_stream.read_line(&mut line) {
            println!("Read data from proxy master failed: {}", e);
            return;
          }
          if line == "\r\n" {
            break;
          }
          lines.push(line);
        }
        if lines.is_empty() {
          println!("Empty data from proxy master");
          continue;
        }
        if lines[0].trim() == "REQUEST" {
          let mut stream = match TcpStream::connect(proxy.master_stream.peer_addr().unwrap()) {
            Ok(stream) => stream,
            Err(e) => {
              println!("Connect to proxy master failed: {}", e);
              continue;
            }
          };
          if let Err(e) = stream
            .write_all(format!("SEND {}\r\n\r\n", self.key).as_bytes())
            .and_then(|_| stream.flush())
          {
            println!("Write to master stream failed: {}", e);
            continue;
          }
          let key = self.key.clone();
          let file_path = file_path.clone();
          let file_name = file_name.clone();
          let mime_type = mime_type.clone();
          thread::spawn(move || {
            Self::handle_send(stream, key, file_path, file_name, is_archive, mime_type);
          });
        }
      }
    } else {
      let listener = match TcpListener::bind(&self.socket) {
        Ok(listener) => listener,
        Err(e) => {
          println!("Bind TCP socket to {} failed: {}", self.socket, e);
          return;
        }
      };
      for stream in listener.incoming() {
        match stream {
          Ok(stream) => {
            let key = self.key.clone();
            let file_path = file_path.clone();
            let file_name = file_name.clone();
            let mime_type = mime_type.clone();
            thread::spawn(move || {
              Self::handle_send(stream, key, file_path, file_name, is_archive, mime_type);
            });
          }
          Err(e) => {
            println!("Get incoming stream failed: {}", e);
            continue;
          }
        }
      }
    }
  }
}
