use std::{
  env,
  fs::{self, File},
  io::{BufRead, BufWriter, Read, Write},
  net::{SocketAddr, TcpListener, TcpStream},
  path::PathBuf,
  process,
  str::FromStr,
  thread,
};

use bufstream::BufStream;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};

use crate::proxy::{ProxyConsumer, ProxyMaster};

static UPLOAD_HTML: &[u8] = include_bytes!("html/upload.html");

pub struct Recv {}

impl Recv {
  pub fn run(key: String, socket: SocketAddr, reserve: bool, proxy: Option<ProxyConsumer>) {
    Self::recv(key, socket, reserve, proxy);
  }

  fn to_os_path(path: &str, reserve: bool) -> PathBuf {
    let path = path.trim();
    if env::consts::OS == "windows" {
      let p = path.replace('/', "\\");
      if reserve && !p.starts_with('\\') {
        PathBuf::from_str(&p).unwrap()
      } else {
        PathBuf::from_str(p.split('\\').last().unwrap()).unwrap()
      }
    } else {
      let p = path.replace('\\', "/");
      if reserve && !p.contains(':') {
        PathBuf::from_str(&p).unwrap()
      } else {
        PathBuf::from_str(p.split('/').last().unwrap()).unwrap()
      }
    }
  }

  fn handle_recv(stream: TcpStream, key: String, reserve: bool) {
    let peer_addr = match stream.peer_addr() {
      Ok(peer_addr) => peer_addr,
      Err(e) => {
        println!("Get peer socket failed: {}", e);
        return;
      }
    };
    let mut buf_stream = BufStream::new(&stream);
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

    if headers[0].trim().starts_with(&format!("GET /{} HTTP/", key)) {
      if let Err(e) = buf_stream
        .write_all(
          format!(
            "HTTP/1.1 200 OK\r\n\
            Content-Type: text/html;charset=utf-8\r\n\
            Content-Length: {}\r\n\
            \r\n",
            UPLOAD_HTML.len()
          )
          .as_bytes(),
        )
        .and_then(|_| buf_stream.write_all(UPLOAD_HTML).and_then(|_| buf_stream.flush()))
      {
        println!("Write response failed: {}", e);
      }
      return;
    }
    if !headers[0].trim().starts_with(&format!("POST /{} HTTP/", key)) {
      println!("Bad Request from {}: {}", peer_addr, headers[0].trim());
      if let Err(e) = buf_stream
        .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
        .and_then(|_| buf_stream.flush())
      {
        println!("Write response header failed: {}", e);
      }
      return;
    }

    if let Err(e) = buf_stream
      .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
      .and_then(|_| buf_stream.flush())
    {
      println!("Write response header failed: {}", e);
      return;
    }

    let mut content_length = None;
    let mut file_path = None;
    for header in headers.into_iter().skip(1) {
      if content_length.is_none() && header.starts_with("Content-Length:") {
        content_length = match header.trim().split(':').take(2).last() {
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
      if file_path.is_none() && header.starts_with("File-Path:") {
        file_path = match header.split(':').take(2).last() {
          Some(path) => Some(Self::to_os_path(path, reserve)),
          None => {
            println!("Get file path failed, Fallback to \"{}\"", key);
            Some(PathBuf::from_str(&key).unwrap())
          }
        };
        if let Some(p) = file_path.as_ref().unwrap().parent() {
          if !p.is_dir() {
            if let Err(e) = fs::create_dir_all(p) {
              println!("Create directory failed: {}", e);
              return;
            }
          }
        }
      }
    }
    let content_length = content_length.unwrap();
    let file_path = file_path.unwrap();
    if file_path.is_file() {
      let mut extension = String::new();
      if let Some(ex) = file_path.extension() {
        extension.push_str(ex.to_str().unwrap());
        extension.push('-');
      }
      extension.push_str(&key);
      let mut bak_path = file_path.clone();
      bak_path.set_extension(&extension);
      let mut i = 0;
      while bak_path.is_file() {
        bak_path.set_extension(format!("{}-{}", extension, i));
        i += 1;
      }
      if let Err(e) = fs::rename(&file_path, &bak_path) {
        println!("Move {:?} to {:?} failed: {}", file_path, bak_path, e);
        return;
      }
      println!("{:?} exists, moved to {:?}", file_path, bak_path);
    }

    println!("\nRecving {:?} from {}", &file_path, peer_addr);
    let pb = ProgressBar::new(content_length);
    pb.set_style(
      ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn std::fmt::Write| {
          write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap()
        })
        .progress_chars("#>-"),
    );

    let mut buf = [0u8; 16 * 1024];
    let mut file_writer = match File::create(&file_path) {
      Ok(file) => BufWriter::new(file),
      Err(e) => {
        println!("Create output file failed: {}", e);
        return;
      }
    };
    let mut left_size = content_length;
    while left_size > 0 {
      match buf_stream.read(&mut buf) {
        Ok(n) => {
          if let Err(e) = file_writer.write_all(&buf[..n]) {
            println!("Write to output file failed: {}", e);
            return;
          }
          pb.inc(n as u64);
          left_size -= n as u64;
        }
        Err(e) => {
          drop(file_writer);
          println!("Read data from stream failed: {}", e);
          fs::remove_file(&file_path).expect("Remove partial received file failed");
          println!("Partial received file has been deleted");
          return;
        }
      }
    }
    if let Err(e) = file_writer.flush() {
      println!("Flush write buffer failed: {}", e);
      return;
    }

    if let Err(e) = buf_stream
      .write_all(b"HTTP/1.1 200 OK\r\n\r\n")
      .and_then(|_| buf_stream.flush())
    {
      println!("Write response header failed: {}", e);
      return;
    }
    println!("Recv {:?} from {} done", file_path, peer_addr);
  }

  fn recv(key: String, socket: SocketAddr, reserve: bool, proxy: Option<ProxyConsumer>) {
    let (pub_addr, proxy_master_socket) = if let Some(proxy) = &proxy {
      (proxy.public_socket, Some(proxy.master_stream.peer_addr().unwrap()))
    } else {
      (socket, None)
    };

    let key_cloned = key.clone();
    if let Err(e) = ctrlc::set_handler(move || {
      if let Some(socket) = proxy_master_socket {
        ProxyMaster::end_proxy(&key_cloned, socket);
      }
      process::exit(0);
    }) {
      println!("Set Ctrl-C handler failed: {}", e);
      return;
    }

    println!();
    println!(
      "cURL (Bash): for f in <FILES>; do curl -X POST -H \"File-Path: $f\" -T $f http://{}/{}; done",
      pub_addr, key
    );
    println!(
      "cURL (PowerShell): foreach ($f in \"f1\", \"f2\") {{ curl -X POST -H \"File-Path: $f\"  -T $f http://{}/{} }}",
      pub_addr, key
    );
    println!(
      "cURL (CMD): FOR %f IN (f1, f2) DO curl -X POST -H \"File-Path: %f\" -T %f http://{}/{}",
      pub_addr, key
    );
    println!("Browser: http://{}/{}", pub_addr, key);

    if let Some(proxy) = proxy {
      let proxy_master_socket = proxy.master_stream.peer_addr().unwrap();
      for stream in ProxyMaster::get_transport_stream(key.clone(), proxy.master_stream) {
        let key = key.clone();
        thread::spawn(move || Self::handle_recv(stream, key, reserve));
      }
      ProxyMaster::end_proxy(&key, proxy_master_socket);
    } else {
      let listener = match TcpListener::bind(&socket) {
        Ok(listener) => listener,
        Err(e) => {
          println!("Bind TCP socket to {} failed: {}", socket, e);
          return;
        }
      };
      for stream in listener.incoming() {
        match stream {
          Ok(stream) => {
            let key = key.clone();
            thread::spawn(move || Self::handle_recv(stream, key, reserve));
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
