// With the default subsystem, 'console', windows creates an additional console window for the program.
// This is silently ignored on non-windows systems.
// See https://msdn.microsoft.com/en-us/library/4cc7ya5b.aspx for more details.
#![windows_subsystem = "windows"]

use std::env;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use getopts::{Options, Matches};
#[allow(unused_imports)]
use log::{debug, error, info, trace, warn, LevelFilter};
use simplelog::*;
#[cfg(windows)]
use winapi::um::wincon::{ATTACH_PARENT_PROCESS, AttachConsole, FreeConsole};

use gis::{Block, Bytes, Chain, Miner, Context, Network, Settings, dns_utils, Keystore, ZONE_DIFFICULTY, GIS_DEBUG, DB_NAME};
use std::fs::OpenOptions;
use std::process::exit;
use std::io::{Seek, SeekFrom};

#[cfg(feature = "webgui")]
mod web_ui;

const SETTINGS_FILENAME: &str = "gis.toml";
const LOG_TARGET_MAIN: &str = "gis::Main";

fn main() {
    // When linked with the windows subsystem windows won't automatically attach
    // to the console of the parent process, so we do it explicitly. This fails silently if the parent has no console.
    #[cfg(windows)]
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
        #[cfg(feature = "webgui")]
        winapi::um::shellscalingapi::SetProcessDpiAwareness(2);
    }

    let args: Vec<String> = env::args().collect();
    let program = args[0].clone();

    let mut opts = Options::new();
    opts.optflag("h", "help", "Print this help menu");
    opts.optflag("n", "nogui", "Run without graphic user interface (default for no gui builds)");
    opts.optflag("v", "version", "Print version and exit");
    opts.optflag("d", "debug", "Show trace messages, more than debug");
    opts.optflag("b", "blocks", "List blocks from DB and exit");
    opts.optflag("g", "generate", "Generate new config file. Generated config will be printed to console.");
    opts.optopt("l", "log", "Write log to file", "FILE");
    opts.optopt("c", "config", "Path to config file", "FILE");
    opts.optopt("w", "work-dir", "Path to working directory", "DIRECTORY");
    opts.optopt("u", "upgrade", "Path to config file that you want to upgrade. Upgraded config will be printed to console.", "FILE");

    let opt_matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => panic!("{}", f.to_string()),
    };

    if opt_matches.opt_present("h") {
        let brief = format!("Usage: {} [options]", program);
        println!("{}", opts.usage(&brief));
        exit(0);
    }

    if opt_matches.opt_present("v") {
        println!("GIS v{}", env!("CARGO_PKG_VERSION"));
        exit(0);
    }

    if opt_matches.opt_present("g") {
        println!("{}", include_str!("../gis.toml"));
        exit(0);
    }

    match opt_matches.opt_str("u") {
        None => {}
        Some(path) => {
            if let Some(settings) = Settings::load(&path) {
                let string = toml::to_string(&settings).unwrap();
                println!("{}", &string);
            } else {
                println!("Error loading config for upgrade!");
            }
            return;
        }
    };

    #[cfg(feature = "webgui")]
    let no_gui = opt_matches.opt_present("n");
    #[cfg(not(feature = "webgui"))]
    let no_gui = true;

    if let Some(path) = opt_matches.opt_str("w") {
        env::set_current_dir(Path::new(&path)).expect(&format!("Unable to change working directory to '{}'", &path));
    }
    let config_name = match opt_matches.opt_str("c") {
        None => { SETTINGS_FILENAME.to_owned() }
        Some(path) => { path }
    };

    setup_logger(&opt_matches);
    info!(target: LOG_TARGET_MAIN, "Starting GIS {}", env!("CARGO_PKG_VERSION"));

    let settings = Settings::load(&config_name).expect(&format!("Cannot load settings from {}!", &config_name));
    debug!(target: LOG_TARGET_MAIN, "Loaded settings: {:?}", &settings);
    let keystore = Keystore::from_file(&settings.key_file, "");
    let mut chain: Chain = Chain::new(&settings, DB_NAME);
    if opt_matches.opt_present("b") {
        for i in 1..(chain.get_height() + 1) {
            if let Some(block) = chain.get_block(i) {
                info!(target: LOG_TARGET_MAIN, "{:?}", &block);
            }
        }
        return;
    }
    chain.check_chain(settings.check_blocks);

    match chain.get_block(1) {
        None => { info!(target: LOG_TARGET_MAIN, "No blocks found in DB"); }
        Some(block) => { trace!(target: LOG_TARGET_MAIN, "Loaded DB with origin {:?}", &block.hash); }
    }
    let settings_copy = settings.clone();
    let context = Context::new(env!("CARGO_PKG_VERSION").to_owned(), settings, keystore, chain);
    let context: Arc<Mutex<Context>> = Arc::new(Mutex::new(context));
    dns_utils::start_dns_server(&context, &settings_copy);

    let mut miner_obj = Miner::new(Arc::clone(&context));
    miner_obj.start_mining_thread();
    let miner: Arc<Mutex<Miner>> = Arc::new(Mutex::new(miner_obj));

    let mut network = Network::new(Arc::clone(&context));
    network.start().expect("Error starting network component");

    create_genesis_if_needed(&context, &miner);
    if no_gui {
        print_my_domains(&context);
        let sleep = Duration::from_millis(1000);
        loop {
            thread::sleep(sleep);
        }
    } else {
        #[cfg(feature = "webgui")]
        web_ui::run_interface(Arc::clone(&context), miner.clone());
    }

    // Without explicitly detaching the console cmd won't redraw it's prompt.
    #[cfg(windows)]
    unsafe {
        FreeConsole();
    }
}

/// Sets up logger in accordance with command line options
fn setup_logger(opt_matches: &Matches) {
    let mut level = LevelFilter::Info;
    if opt_matches.opt_present("d") || env::var(GIS_DEBUG).is_ok() {
        level = LevelFilter::Trace;
    }
    let config = ConfigBuilder::new()
        .add_filter_ignore_str("mio::poll")
        .set_thread_level(LevelFilter::Off)
        .set_location_level(LevelFilter::Off)
        .set_target_level(LevelFilter::Error)
        .set_time_level(LevelFilter::Error)
        .set_time_to_local(true)
        .build();
    match opt_matches.opt_str("l") {
        None => {
            if let Err(e) = TermLogger::init(level, config, TerminalMode::Stdout, ColorChoice::Auto) {
                println!("Unable to initialize logger!\n{}", e);
            }
        }
        Some(path) => {
            let file = match OpenOptions::new().write(true).create(true).open(&path) {
                Ok(mut file) => {
                    file.seek(SeekFrom::End(0)).unwrap();
                    file
                }
                Err(e) => {
                    println!("Could not open log file '{}' for writing!\n{}", &path, e);
                    exit(1);
                }
            };
            CombinedLogger::init(
                vec![
                    TermLogger::new(level, config.clone(), TerminalMode::Stdout, ColorChoice::Auto),
                    WriteLogger::new(level, config, file),
                ]
            ).unwrap();
        }
    }
}

/// Gets own domains by current loaded keystore and writes them to log
fn print_my_domains(context: &Arc<Mutex<Context>>) {
    let context = context.lock().unwrap();
    let domains = context.chain.get_my_domains(&context.keystore);
    debug!("Domains: {:?}", &domains);
}

/// Creates genesis (origin) block if `origin` is empty in config and we don't have any blocks in DB
fn create_genesis_if_needed(context: &Arc<Mutex<Context>>, miner: &Arc<Mutex<Miner>>) {
    // If there is no origin in settings and no blockchain in DB, generate genesis block
    let context = context.lock().unwrap();
    let last_block = context.get_chain().last_block();
    let origin = context.settings.origin.clone();
    if origin.is_empty() && last_block.is_none() {
        if let Some(keystore) = &context.keystore {
            // If blockchain is empty, we are going to mine a Genesis block
            let block = Block::new(None, context.get_keystore().unwrap().get_public(), Bytes::default(), ZONE_DIFFICULTY);
            miner.lock().unwrap().add_block(block, keystore.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use alfis::dns::protocol::{DnsRecord, TransientTtl};

    #[test]
    fn record_to_string() {
        let record = DnsRecord::A {
            domain: "google.com".to_string(),
            addr: "127.0.0.1".parse().unwrap(),
            ttl: TransientTtl(300)
        };
        println!("Record is {:?}", &record);
        println!("Record in JSON is {}", serde_json::to_string(&record).unwrap());
    }
}
