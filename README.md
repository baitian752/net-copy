# Net Copy

A simple command line tool to transfer files with HTTP

![example](assets/example.gif)

# Usage

## Help

```text
$ A simple command line tool to transfer files with HTTP

Usage: ncp [OPTIONS] [FILES]...

Arguments:
  [FILES]...  The files to be sent, empty means serve as receiver

Options:
  -l, --host <HOST>    The host ip for the server
  -p, --port <PORT>    The port for the server
  -k, --key <STRING>   The secret key for the server
  -r, --reserve        Whether reserve the full path of the received file
  -x, --proxy <PROXY>  Proxy for TCP connection
  -X, --no-proxy       Disable automatically check proxy from gateway
  -m, --mode <MODE>    Serve mode [possible values: normal, proxy]
  -h, --help           Print help information
  -V, --version        Print version information
```

The options will first parse from command line, then from environment variables (env), finally from config file.

The env name is `NCP_<UPPER CASE OF OPTION>`, e.g. `NCP_KEY`.

The config file path may be `~/.config/ncp.toml` or `/etc/ncp.toml` (Unix-like), `%APPDATA%\ncp.toml` (Windows), the first has higher priority.

## Send

### One file

```text
$ ncp `which ncp`

cURL: curl -o ncp http://172.17.0.8:24232/H95kvE
Wget: wget -O ncp http://172.17.0.8:24232/H95kvE
PowerShell: iwr -O ncp http://172.17.0.8:24232/H95kvE
Browser: http://172.17.0.8:24232/H95kvE
```

### Multiple files

```text
$ ncp `ls`

cURL: curl http://172.17.0.8:18382/HUZ1iR | tar xvf -
Wget: wget -O- http://172.17.0.8:18382/HUZ1iR | tar xvf -
PowerShell: cmd /C 'curl http://172.17.0.8:18382/HUZ1iR | tar xvf -'
Browser: http://172.17.0.8:18382/HUZ1iR
```

## Receive

```text
$ ncp

cURL (Bash): for f in <FILES>; do curl -X POST -H "File-Path: $f" -T $f http://172.17.0.8:15962/giSY01; done
cURL (PowerShell): foreach ($f in "f1", "f2") { curl -X POST -H "File-Path: $f" -T $f http://172.17.0.8:15962/giSY01 }
cURL (CMD): FOR %f IN (f1, f2) DO curl -X POST -H "File-Path: %f" -T %f http://172.17.0.8:15962/giSY01
Browser: http://172.17.0.8:15962/giSY01
```

## Proxy

The proxy works as a server, and the sender/receiver detect proxy automatically from all network gateways, or you can specific proxy servers from cli/env/config.

# Note

- If you get an error related to [glibc](https://www.gnu.org/software/libc/), please use the [musl](https://musl.libc.org/) version, which is static linking
- For android, you can use the `-unknown-linux-musl` version
