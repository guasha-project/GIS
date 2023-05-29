# ![](/img/logo/32px.png) Gis

![Builds](https://github.com/Revertron/Alfis/actions/workflows/rust_build_and_test.yml/badge.svg)

Guasha Identity System

This project represents a minimal blockchain without cryptocurrency, capable of sustaining any number of domain name zones and domains.



## Building and running

### On every OS
You can download and run already built binaries from [releases](https://github.com/guasha-project/GIS/releases), or you can build project yourself.

You can build Gis by issuing `cargo build` and `cargo run` commands in a directory of cloned repository.
If you want to build release version you need to do `cargo build --release` as usual.

### ![Windows Logo](/img/windows.svg) On Windows
You don't need any additional steps to build Gis, just stick to the MSVC version of Rust.

If you see an error about missing `VCRUNTIME140.dll` when running gis you will need to install [VC Redistributable](https://www.microsoft.com/en-us/download/details.aspx?id=52685) from Microsoft.

If you want to use modern browser engine from Edge instead of old from IE, you need to build with this command: `cargo build --release --features "edge"` (or use corresponding build from [releases](https://github.com/guasha-project/GIS/releases)).

### ![Windows Logo](/img/windows.svg) On Windows (MINGW64)
If you'd rather use Gnu version of Rust you can build Gis by these steps:
```
pacman -S git mingw64/mingw-w64-x86_64-rust mingw64/mingw-w64-x86_64-cargo-c
git clone https://github.com/guasha-project/GIS.git
cd Gis
cargo build
```

### ![Linux Logo](/img/linux.svg) On Linux
If you are building on Linux you must ensure that you have `libwebkitgtk` library installed.
You can do it by issuing this command: `sudo apt install libwebkit2gtk-4.0-dev` (on Debian/Ubuntu and derivatives).

#### ![Arch Linux Logo](/img/archlinux.svg) On Arch Linux

Create and install package with this commands:

```sh
# make package
git clone https://github.com/guasha-project/GIS.git
cd Gis/contrib
makepkg

# install package (from root)
pacman -U gis-<version>-1-x86_64.pkg.tar.xz
```

## Installation

### Debian/Ubuntu (only blockchain DNS, without GUI)
If you want to just use GIS as a DNS daemon and resolve domains in blockchain, as well as clearnet domains.
You just need to install `gis` service from repo and change your resolver in `/etc/resolv.conf`.
Beware of NetworkManager, it can change your resolvers at will.

1. Download repository public key and add it to your APT
```
wget -O - https://deb.revertron.com/key.txt | sudo apt-key add -
```
2. Add repository path to sources list
```
echo 'deb http://deb.revertron.com/ debian alfis' | sudo tee /etc/apt/sources.list.d/alfis.list
```
3. Update packages
```
sudo apt update
```
4. Install GIS
```
sudo apt install gis
```
After that configuration is in file `/etc/gis.conf` and data is saved to `/var/lib/gis`.
If you have some DNS server bound to port 53, it will not properly start. Deal with it on your own.

### GUI version Windows/Linux/MacOS (if you want to create and change domains)
If you want to create and manage your own domains on blockchain, you will need a version with GUI.
You can download it from [releases](https://github.com/guasha-project/gis/releases) section, choose appropriate OS and architecture version.
It needs to be without `nogui` suffix.

Just unzip that archive in some directory and run `gis` (or `gis.exe`) binary.
By default, it searches for config file, named `gis.toml` in current working directory, and creates/changes `guachain.db` file in the same directory.
If you want it to load config from another file you can command it so: `gis -c /etc/gis.conf`.
