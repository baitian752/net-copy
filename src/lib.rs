use std::{
  env,
  fs::{self, File},
  io::{BufRead, BufReader, BufWriter, Read, Write},
  net::{SocketAddr, TcpListener},
  path::{Path, PathBuf},
  process,
  str::FromStr,
  thread,
};

use clap::ValueEnum;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use serde_derive::{Deserialize, Serialize};
use tar::Builder;

#[derive(ValueEnum, Clone, Debug, Serialize, Deserialize)]
pub enum Mode {
  Normal,
  Proxy,
}

pub struct NetCopy {
  mode: Mode,
  key: String,
  file_paths: Vec<PathBuf>,
  listener: TcpListener,
  upload_html: &'static [u8],
  reserve: bool,
}

impl NetCopy {
  pub fn new(
    mode: Mode, file_paths: Vec<PathBuf>, key: &str, socket: SocketAddr, upload_html: &'static [u8], reserve: bool,
  ) -> Self {
    NetCopy {
      mode,
      key: key.into(),
      file_paths,
      listener: TcpListener::bind(&socket).unwrap_or_else(|e| panic!("Bind TCP socket to {} failed: {:?}", socket, e)),
      upload_html,
      reserve,
    }
  }

  pub fn run(&self) {
    match self.mode {
      Mode::Normal => {
        if self.file_paths.is_empty() {
          self.recv()
        } else {
          self.send()
        }
      }
      Mode::Proxy => {
        if !self.file_paths.is_empty() {
          println!("WARNING: The proxy mode has activated, files will be ignored");
        }
        self.proxy();
      }
    }
  }

  fn to_os_path(path: &str, reserve: bool) -> PathBuf {
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

  fn archive(&self, tar_path: &Path) {
    let mut tar = Builder::new(File::create(tar_path).expect("Create tar failed"));
    for file_path in &self.file_paths {
      if file_path.is_dir() {
        tar.append_dir_all(file_path, file_path)
      } else {
        tar.append_path(file_path)
      }
      .expect("Append file to tar failed");
    }
    tar.finish().expect("Write tar file failed");
  }

  fn send(&self) {
    let (file_path, is_archive) = if self.file_paths.len() == 1 && self.file_paths[0].is_file() {
      (self.file_paths[0].clone(), false)
    } else {
      let tar_path = PathBuf::from_str(&format!("{}.tar", self.key)).unwrap();
      self.archive(&tar_path);
      (tar_path, true)
    };
    let file_name = file_path.file_name().unwrap().to_str().unwrap();
    let socket = self.listener.local_addr().unwrap();
    let tar_path = file_path.clone();
    ctrlc::set_handler(move || {
      if is_archive && tar_path.is_file() {
        fs::remove_file(&tar_path).expect("Remove tar file failed");
      }
      process::exit(0);
    })
    .expect("Set Ctrl-C handler failed");
    println!();
    if is_archive {
      println!("cURL: curl http://{}/{} | tar xvf -", socket, self.key);
      println!("Wget: wget -O- http://{}/{} | tar xvf -", socket, self.key);
      println!("PowerShell: cmd /C 'curl http://{}/{} | tar xvf -'", socket, self.key);
    } else {
      println!("cURL: curl -o {} http://{}/{}", file_name, socket, self.key);
      println!("Wget: wget -O {} http://{}/{}", file_name, socket, self.key);
      println!("PowerShell: iwr -O {} http://{}/{}", file_name, socket, self.key);
    }
    println!("Browser: http://{}/{}", socket, self.key);
    println!();
    for stream in self.listener.incoming() {
      let file_name = file_name.to_string();
      let key = self.key.clone();
      match stream {
        Ok(mut stream) => {
          let file_path = file_path.clone();
          if !file_path.is_file() && is_archive {
            self.archive(&file_path);
          }
          thread::spawn(move || {
            let peer_addr = stream.peer_addr().expect("Get peer socket failed");
            let mut buf_reader = BufReader::new(&mut stream);
            let mut request_line = String::new();
            buf_reader
              .read_line(&mut request_line)
              .expect("Read line from buffer failed");
            if request_line.trim() != format!("GET /{} HTTP/1.1", key) {
              stream
                .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                .expect("Write response header failed");
              println!("Bad Request (from {}): {}", peer_addr, request_line.trim());
              return;
            }
            println!("\nSending {:?} to {}", file_path, peer_addr);
            let mut f = File::open(&file_path).expect("Open sending file failed");
            let file_size = f.metadata().expect("Get file size metadata failed").len();

            let response = format!(
              "HTTP/1.1 200 OK\r\n\
              Content-Length: {file_size}\r\n\
              Content-Type: application/octet-stream\r\n\
              Content-Disposition: attachment; filename=\"{file_name}\"\r\n\
              \r\n"
            );
            stream
              .write_all(response.as_bytes())
              .expect("Write response header failed");

            const CHUNK_SIZE: u64 = 8 * 1024;
            let mut buf = vec![0u8; CHUNK_SIZE as usize];
            for _ in 0..(file_size / CHUNK_SIZE) {
              f.read_exact(&mut buf).expect("Read sending file failed");
              stream.write_all(&buf).expect("Write response content failed");
            }
            let left_size = file_size % CHUNK_SIZE;
            if left_size != 0 {
              buf.resize(left_size as usize, 0u8);
              f.read_exact(&mut buf).expect("Read sending file failed");
              stream.write_all(&buf).expect("Write response content failed");
            }
            println!("Send {:?} to {} done", file_path, peer_addr);
          });
        }
        Err(e) => panic!("Error: {:?}", e),
      }
    }
  }

  fn recv(&self) {
    let socket = self.listener.local_addr().unwrap();
    println!();
    println!(
      "cURL (Bash): for f in <FILES>; do curl -X POST -H \"File-Path: $f\" -T $f http://{}/{}; done",
      socket, self.key
    );
    println!(
      "cURL (PowerShell): foreach ($f in \"f1\", \"f2\") {{ curl -X POST -H \"File-Path: $f\" -T $f http://{}/{} }}",
      socket, self.key
    );
    println!(
      "cURL (CMD): FOR %f IN (f1, f2) DO curl -X POST -H \"File-Path: %f\" -T %f http://{}/{}",
      socket, self.key
    );
    println!("Browser: http://{}/{}", socket, self.key);
    println!();
    let upload_html = self.upload_html;
    let reserve = self.reserve;
    for stream in self.listener.incoming() {
      let key = self.key.clone();
      match stream {
        Ok(mut stream) => {
          thread::spawn(move || {
            let peer_addr = stream.peer_addr().expect("Get peer socket failed");
            let mut buf_reader = BufReader::new(&mut stream);
            let mut request_line = String::new();
            buf_reader
              .read_line(&mut request_line)
              .expect("Read line from buffer failed");
            if request_line.trim() == format!("GET /{} HTTP/1.1", key) {
              stream
                .write_all(
                  b"HTTP/1.1 200 OK\r\n\
                  Content-Type: text/html;charset=utf-8\r\n\
                  \r\n",
                )
                .expect("Write response header failed");
              stream.write_all(upload_html).expect("Write response body failed");
              return;
            }
            if request_line.trim() != format!("POST /{} HTTP/1.1", key) {
              stream
                .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                .expect("Write response header failed");
              println!("Bad Request (from {}): {}", peer_addr, request_line.trim());
              return;
            }
            let mut line = String::new();
            let mut content_length = 0;
            let mut file_path: Option<PathBuf> = None;
            let mut should_send_continue_response = false;
            loop {
              buf_reader.read_line(&mut line).expect("Read line from buffer failed");
              println!("{}", line.trim());
              if line == "\r\n" {
                break;
              }
              if line.starts_with("Content-Length:") {
                content_length = line
                  .split(':')
                  .take(2)
                  .last()
                  .expect("Parse content length failed")
                  .trim()
                  .parse::<u64>()
                  .expect("Parse content length to unsigned integer failed");
              }
              if line.starts_with("File-Path:") {
                let path = Self::to_os_path(
                  line.split(':').take(2).last().expect("Parst file path failed").trim(),
                  reserve,
                );
                if let Some(p) = path.parent() {
                  if !p.is_dir() {
                    fs::create_dir_all(p).expect("Create directory failed");
                  }
                }
                file_path = Some(path);
              }
              if line.trim() == "Expect: 100-continue" {
                should_send_continue_response = true;
              }
              line.clear();
            }
            if content_length == 0 {
              panic!("Content length is 0");
            }
            if file_path.is_none() {
              panic!("File path is empty");
            }
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
              fs::rename(&file_path, &bak_path)
                .unwrap_or_else(|e| panic!("Move {:?} to {:?} failed: {:?}", file_path, bak_path, e));
              println!("{:?} exists, moved to {:?}", file_path, bak_path);
            }
            if should_send_continue_response {
              stream
                .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
                .expect("Write response header failed");
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
            const CHUNK_SIZE: u64 = 8 * 1024;
            let mut buf = [0u8; CHUNK_SIZE as usize];
            let mut buf_reader = BufReader::new(&mut stream);
            let mut buf_writer = BufWriter::new(File::create(&file_path).expect("Create upload file failed"));
            let mut left_size = content_length;
            while left_size > 0 {
              match buf_reader.read(&mut buf) {
                Ok(n) => {
                  if n == 0 {
                    println!("{} has closed the connection", peer_addr);
                    return;
                  }
                  buf_writer.write_all(&buf[0..n]).expect("Write to upload file failed");
                  pb.inc(n as u64);
                  left_size -= n as u64;
                }
                Err(_) => {
                  drop(buf_writer);
                  fs::remove_file(&file_path).expect("Remove partial received file failed");
                  panic!("Read data from stream failed, partial received file has been deleted");
                }
              }
            }
            buf_writer.flush().expect("Flush write buffer failed");

            stream
              .write_all(b"HTTP/1.1 200 OK\r\n\r\n")
              .expect("Write response header failed");
            println!("Recv {:?} from {} done", &file_path, peer_addr);
          });
        }
        Err(e) => panic!("Error: {:?}", e),
      }
    }
  }

  fn proxy(&self) {
    todo!();
    // let socket = self.listener.local_addr().unwrap();
    // println!("Net Copy: ncp -x {} -k {} [FILES]", socket, self.key);
    // for stream in self.listener.incoming() {
    //   let key = self.key.clone();
    // }
  }
}
