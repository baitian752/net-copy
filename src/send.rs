use std::{
  fs::{self, File},
  io::{BufRead, BufReader, Read, Write},
  net::{SocketAddr, TcpListener, TcpStream},
  path::{Path, PathBuf},
  process,
  sync::Mutex,
  thread,
};

use base64::{engine::general_purpose, Engine as _};
use bufstream::BufStream;
use mime_guess;
use tar::Builder;

use crate::proxy::{ProxyConsumer, ProxyMaster};

static FILE_PATHS: Mutex<Vec<PathBuf>> = Mutex::new(vec![]);

pub struct Send {}

impl Send {
  pub fn run(key: &str, socket: SocketAddr, proxy: Option<ProxyConsumer>, file_paths: Vec<PathBuf>) {
    FILE_PATHS.lock().unwrap().clear();
    for file_path in file_paths {
      FILE_PATHS.lock().unwrap().push(file_path);
    }
    Self::send(&key, socket, proxy);
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
    stream: TcpStream, key: &str, file_path: PathBuf, file_name: String, is_archive: bool, mime_type: String,
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
      if headers.len() > 100 {
        println!("Too many request headers from {}", peer_addr);
        return;
      }
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
      }
      return;
    }

    println!("\nSending {:?} to {}", file_path, peer_addr);
    let (file_size, mut file_reader) = match File::open(&file_path) {
      Ok(file) => (
        match file.metadata() {
          Ok(metadata) => metadata.len() as usize,
          Err(e) => {
            println!("Get file metadata failed: {}", e);
            return;
          }
        },
        BufReader::new(file),
      ),
      Err(e) => {
        println!("Open file {:?} failed: {}", file_path, e);
        return;
      }
    };

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
    let mut send_size = 0;
    while left_size > 0 {
      match file_reader.read(&mut buf) {
        Ok(n) => {
          if let Err(e) = buf_stream.write_all(&buf[..n]) {
            println!("Write response content failed: {}", e);
            return;
          }
          left_size -= n;
          send_size += n;
          if send_size >= 16 * 1024 * 1024 {
            if let Err(e) = buf_stream.flush() {
              println!("Flush writer failed: {}", e);
              return;
            }
            send_size = 0;
          }
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
    println!("Send {:?} to {} done", file_path, peer_addr);
  }

  fn send(key: &str, socket: SocketAddr, proxy: Option<ProxyConsumer>) {
    let is_archive = Self::is_archive();
    let file_path = if is_archive {
      let file_path = Self::get_tar_path(key);
      if !file_path.is_file() {
        Self::tar(&file_path);
      }
      file_path
    } else {
      FILE_PATHS.lock().unwrap()[0].clone()
    };
    let file_name = file_path.file_name().unwrap().to_str().unwrap().to_string();
    let mime_type = mime_guess::from_path(&file_path).first_or_octet_stream().to_string();

    let (pub_addr, proxy_master_socket) = if let Some(proxy) = &proxy {
      (proxy.public_socket, Some(proxy.master_stream.peer_addr().unwrap()))
    } else {
      (socket, None)
    };

    let tar_path = file_path.clone();
    let key_cloned = key.to_string();
    if let Err(e) = ctrlc::set_handler(move || {
      if let Some(socket) = proxy_master_socket {
        ProxyMaster::end_proxy(&key_cloned, socket);
      }
      if is_archive && tar_path.is_file() {
        fs::remove_file(&tar_path).expect("Remove tar file failed");
      }
      process::exit(0);
    }) {
      println!("Set Ctrl-C handler failed: {}", e);
      return;
    }

    println!();
    let default_cmd;
    if is_archive {
      default_cmd = format!("curl http://{}/{} | tar xvf -", pub_addr, key);
      print!(
        "\x1B]52;c;{}\x07",
        general_purpose::STANDARD_NO_PAD.encode(&default_cmd)
      );
      println!("cURL: {}", default_cmd);
      println!("Wget: wget -O- http://{}/{} | tar xvf -", pub_addr, key);
    } else {
      default_cmd = format!("curl -o \"{}\" http://{}/{}", file_name, pub_addr, key);
      print!(
        "\x1B]52;c;{}\x07",
        general_purpose::STANDARD_NO_PAD.encode(&default_cmd)
      );
      println!("cURL: {}", default_cmd);
      println!("Wget: wget -O \"{}\" http://{}/{}", file_name, pub_addr, key);
    }

    if let Some(proxy) = proxy {
      let proxy_master_socket = proxy.master_stream.peer_addr().unwrap();
      for stream in ProxyMaster::get_transport_stream(key, proxy.master_stream) {
        let key = key.to_string();
        let file_path = file_path.clone();
        let file_name = file_name.clone();
        let mime_type = mime_type.clone();
        thread::spawn(move || {
          Self::handle_send(stream, &key, file_path, file_name, is_archive, mime_type);
        });
      }
      ProxyMaster::end_proxy(&key, proxy_master_socket);
    } else {
      let listener = match TcpListener::bind(socket) {
        Ok(listener) => listener,
        Err(e) => {
          println!("Bind TCP socket to {} failed: {}", socket, e);
          return;
        }
      };
      for stream in listener.incoming() {
        match stream {
          Ok(stream) => {
            let key = key.to_string();
            let file_path = file_path.clone();
            let file_name = file_name.clone();
            let mime_type = mime_type.clone();
            thread::spawn(move || {
              Self::handle_send(stream, &key, file_path, file_name, is_archive, mime_type);
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
