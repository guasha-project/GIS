use std::fs::File;
use std::io::Read;

use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use log::{debug, error, info, LevelFilter, trace, warn};

use crate::Bytes;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub key_file: String,
    #[serde(default = "default_check_blocks")]
    pub check_blocks: u64,
    #[serde(default)]
    pub net: Net,
    #[serde(default)]
    pub dns: Dns,
    #[serde(default)]
    pub mining: Mining,
}

impl Settings {
    pub fn load(filename: &str) -> Option<Settings> {
        match File::open(filename) {
            Ok(mut file) => {
                let mut text = String::new();
                file.read_to_string(&mut text).unwrap();
                if let Ok(settings) = toml::from_str(&text) {
                    return Some(settings);
                }
                None
            }
            Err(..) => {
                None
            }
        }
    }

    pub fn get_origin(&self) -> Bytes {
        if self.origin.eq("") {
            return Bytes::zero32();
        }
        let origin = crate::from_hex(&self.origin).expect("Wrong origin in settings");
        Bytes::from_bytes(origin.as_slice())
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            origin: String::from(""),
            key_file: String::from("default.key"),
            check_blocks: default_check_blocks(),
            net: Net::default(),
            dns: Default::default(),
            mining: Mining::default()
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dns {
    #[serde(default = "default_listen_dns")]
    pub listen: String,
    #[serde(default = "default_threads")]
    pub threads: usize,
    pub forwarders: Vec<String>,
    #[serde(default)]
    pub hosts: Vec<String>,
}

impl Default for Dns {
    fn default() -> Self {
        Dns {
            listen: String::from("127.0.0.1:53"),
            threads: 20,
            forwarders: vec![String::from("94.140.14.14:53"), String::from("94.140.15.15:53")],
            hosts: Vec::new()
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Mining {
    #[serde(default)]
    pub threads: usize,
    #[serde(default)]
    pub lower: bool
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Net {
    #[serde(default)]
    pub peers: Vec<String>,
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default)]
    pub public: bool,
    #[serde(default)]
    pub yggdrasil_only: bool,
}

impl Default for Net {
    fn default() -> Self {
        Net {
            peers: vec![String::from(""), String::from("")],
            listen: String::from("[::]:46866"),
            public: true,
            yggdrasil_only: false
        }
    }
}

fn default_listen() -> String {
    String::from("[::]:46866")
}

fn default_listen_dns() -> String {
    String::from("0.0.0.0:53")
}

fn default_threads() -> usize {
    100
}

fn default_check_blocks() -> u64 {
    8
}
