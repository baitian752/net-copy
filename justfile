default: x86_64

x86_64: (build "x86_64")
aarch64: (build "aarch64")

build target:
  RUSTFLAGS="-C target-feature=+crt-static" cross build --target {{target}}-unknown-linux-musl --release

tag version:
  git tag -a {{ version }} -m "Bump to {{ version }}"

