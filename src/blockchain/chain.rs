use std::cell::RefCell;
use std::collections::{HashSet, HashMap};
use std::fs;
use std::path::Path;

use chrono::Utc;
#[allow(unused_imports)]
use log::{debug, error, info, trace, warn};
use sqlite::{Connection, State, Statement};

use crate::{Block, Bytes, Keystore, Transaction, check_domain, get_domain_zone, is_yggdrasil_record};
use crate::commons::constants::*;
use crate::blockchain::types::{BlockQuality, MineResult, Options};
use crate::blockchain::types::BlockQuality::*;
use crate::blockchain::hash_utils::*;
use crate::settings::Settings;
use crate::keys::check_public_key_strength;
use std::cmp::max;
use crate::blockchain::transaction::{ZoneData, DomainData};
use std::ops::Deref;
use crate::blockchain::types::MineResult::*;

const TEMP_DB_NAME: &str = "temp.db";
const SQL_CREATE_TABLES: &str = include_str!("sql/create_db.sql");
const SQL_ADD_BLOCK: &str = "INSERT INTO blocks (id, timestamp, version, difficulty, random, nonce, 'transaction',\
                          prev_block_hash, hash, pub_key, signature) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?);";
const SQL_GET_LAST_BLOCK: &str = "SELECT * FROM blocks ORDER BY id DESC LIMIT 1;";
const SQL_TRUNCATE_BLOCKS: &str = "DELETE FROM blocks WHERE id >= ?;";
const SQL_TRUNCATE_DOMAINS: &str = "DELETE FROM domains WHERE id >= ?;";
const SQL_TRUNCATE_ZONES: &str = "DELETE FROM zones WHERE id >= ?;";

const SQL_ADD_DOMAIN: &str = "INSERT INTO domains (id, timestamp, identity, confirmation, data, pub_key) VALUES (?, ?, ?, ?, ?, ?)";
const SQL_ADD_ZONE: &str = "INSERT INTO zones (id, timestamp, identity, confirmation, data, pub_key) VALUES (?, ?, ?, ?, ?, ?)";
const SQL_GET_BLOCK_BY_ID: &str = "SELECT * FROM blocks WHERE id=? LIMIT 1;";
const SQL_GET_LAST_FULL_BLOCK: &str = "SELECT * FROM blocks WHERE id < ? AND `transaction`<>'' ORDER BY id DESC LIMIT 1;";
const SQL_GET_LAST_FULL_BLOCK_FOR_KEY: &str = "SELECT * FROM blocks WHERE id < ? AND `transaction`<>'' AND pub_key = ? ORDER BY id DESC LIMIT 1;";
const SQL_GET_DOMAIN_PUBLIC_KEY_BY_ID: &str = "SELECT pub_key FROM domains WHERE id < ? AND identity = ? LIMIT 1;";
const SQL_GET_ZONE_PUBLIC_KEY_BY_ID: &str = "SELECT pub_key FROM zones WHERE id < ? AND identity = ? LIMIT 1;";
const SQL_GET_DOMAIN_BY_ID: &str = "SELECT * FROM domains WHERE identity = ? ORDER BY id DESC LIMIT 1;";
const SQL_GET_DOMAINS_BY_KEY: &str = "SELECT * FROM domains WHERE pub_key = ?;";
const SQL_GET_ZONES: &str = "SELECT data FROM zones;";

const SQL_GET_OPTIONS: &str = "SELECT * FROM options;";

/// Max possible block index
const MAX:u64 = i64::MAX as u64;

pub struct Chain {
    origin: Bytes,
    last_block: Option<Block>,
    last_full_block: Option<Block>,
    max_height: u64,
    db: Connection,
    zones: RefCell<HashSet<String>>,
    signers: RefCell<SignersCache>,
}

impl Chain {
    pub fn new(settings: &Settings, db_name: &str) -> Self {
        let origin = settings.get_origin();

        let db = sqlite::open(db_name).expect("Unable to open blockchain DB");
        let zones = RefCell::new(HashSet::new());
        let mut chain = Chain { origin, last_block: None, last_full_block: None, max_height: 0, db, zones, signers: SignersCache::new() };
        chain.init_db();
        chain
    }

    /// Reads options from DB or initializes and writes them to DB if not found
    fn init_db(&mut self) {
        let options = self.get_options();
        if !self.origin.is_zero() && !options.origin.is_empty() && self.origin.to_string() != options.origin {
            self.clear_db();
        }
        if options.version < DB_VERSION {
            self.migrate_db(options.version, DB_VERSION);
        }

        // Trying to get last block from DB to check its version
        // If some block loaded we check its version and determine if we need some migration
        if let Some(block) = self.load_last_block() {
            // Cache some info
            self.last_block = Some(block.clone());
            if block.transaction.is_some() {
                self.last_full_block = Some(block);
            } else {
                self.last_full_block = self.get_last_full_block(MAX, None);
            }
        }
    }

    pub fn check_chain(&mut self, count: u64) {
        let height = self.get_height();
        let start = if height > count {
            info!("Checking last {} blocks...", count);
            height - count + 1
        } else {
            info!("Local blockchain height is {}, starting full blockchain check...", height);
            1
        };
        let mut last_block: Option<Block> = None;
        let mut last_full_block: Option<Block> = None;
        if start > 1 {
            last_block = self.get_block(start - 1);
            if let Some(last) = &last_block {
                last_full_block = match &last.transaction {
                    None => { self.get_last_full_block(last.index, None) }
                    Some(_) => { Some(last.clone()) }
                };
            }
        }

        for id in start..=height {
            debug!("Checking block {}", id);
            let block = self.get_block(id);
            match block {
                None => {
                    panic!("Blockchain is corrupted! Please, delete 'guachain.db' and restart.");
                }
                Some(block) => {
                    if block.index == 1 {
                        if block.hash != self.origin {
                            panic!("Loaded DB is not of origin {:?}! Please, delete 'guachain.db' and restart.", &self.origin);
                        }
                        debug!("Block {} with hash {:?} is good!", block.index, &block.hash);
                        last_block = Some(block);
                        continue;
                    }

                    //let last = self.last_block.clone().unwrap();
                    if self.check_block(&block, &last_block, &last_full_block) != BlockQuality::Good {
                        error!("Block {} is bad:\n{:?}", block.index, &block);
                        info!("Truncating database from block {}...", block.index);
                        match self.truncate_db_from_block(block.index) {
                            Ok(_) => {}
                            Err(e) => {
                                error!("{}", e);
                                panic!("Error truncating database! Please, delete 'guachain.db' and restart.");
                            }
                        }
                        break;
                    }
                    debug!("Block {} with hash {:?} is good!", block.index, &block.hash);
                    if block.transaction.is_some() {
                        self.last_full_block = Some(block.clone());
                    }
                    if block.transaction.is_some() {
                        last_full_block = Some(block.clone());
                    }
                    last_block = Some(block);
                }
            }
        }
        self.last_block = self.load_last_block();
        self.last_full_block = self.get_last_full_block(MAX, None);
        debug!("Last block after chain check: {:?}", &self.last_block);
    }

    fn truncate_db_from_block(&mut self, index: u64) -> sqlite::Result<State> {
        let mut statement = self.db.prepare(SQL_TRUNCATE_BLOCKS)?;
        statement.bind(1, index as i64)?;
        statement.next()?;

        let mut statement = self.db.prepare(SQL_TRUNCATE_DOMAINS)?;
        statement.bind(1, index as i64)?;
        statement.next()?;

        let mut statement = self.db.prepare(SQL_TRUNCATE_ZONES)?;
        statement.bind(1, index as i64)?;
        statement.next()
    }

    fn load_last_block(&mut self) -> Option<Block> {
        match self.db.prepare(SQL_GET_LAST_BLOCK) {
            Ok(mut statement) => {
                let mut result = None;
                while statement.next().unwrap() == State::Row {
                    match Self::get_block_from_statement(&mut statement) {
                        None => {
                            error!("Something wrong with block in DB!");
                            panic!();
                        }
                        Some(block) => {
                            debug!("Loaded last block: {:?}", &block);
                            result = Some(block);
                            break;
                        }
                    }
                }
                result
            }
            Err(e) => {
                info!("No blockchain database found. Creating new. {}", e);
                self.db.execute(SQL_CREATE_TABLES).expect("Error creating DB tables");
                None
            }
        }
    }

    fn migrate_db(&mut self, from: u32, to: u32) {
        debug!("Migrating DB from {} to {}", from, to);
    }

    fn clear_db(&mut self) {
        warn!("Clearing DB");
        // We cannot close DB connection and recreate file,
        // therefore we switch our db to temporary file, delete main DB and switch back.
        // I know that this is a crutch, but this way I don't need to use Option<db> :)
        self.db = sqlite::open(TEMP_DB_NAME).expect("Unable to open temporary blockchain DB");
        let file = Path::new(DB_NAME);
        if fs::remove_file(&file).is_err() {
            panic!("Unable to remove database!");
        }
        self.db = sqlite::open(DB_NAME).expect("Unable to open blockchain DB");
        let file = Path::new(TEMP_DB_NAME);
        let _ = fs::remove_file(&file).is_err();
    }

    fn get_options(&self) -> Options {
        let mut options = Options::empty();
        if let Ok(mut statement) = self.db.prepare(SQL_GET_OPTIONS) {
            while let State::Row = statement.next().unwrap() {
                let name = statement.read::<String>(0).unwrap();
                let value = statement.read::<String>(1).unwrap();
                match name.as_ref() {
                    "origin" => options.origin = value,
                    "version" => options.version = value.parse().unwrap(),
                    _ => {}
                }
            }
        }
        options
    }

    pub fn add_block(&mut self, block: Block) {
        debug!("Adding block:\n{:?}", &block);
        let index = block.index;
        let timestamp = block.timestamp;
        self.last_block = Some(block.clone());
        if block.transaction.is_some() {
            self.last_full_block = Some(block.clone());
        }
        let transaction = block.transaction.clone();
        if self.add_block_to_table(block).is_ok() {
            if let Some(transaction) = transaction {
                self.add_transaction_to_table(index, timestamp, &transaction).expect("Error adding transaction");
            }
        }
    }

    pub fn replace_block(&mut self, block: Block) -> sqlite::Result<()> {
        warn!("Replacing block {} with:\n{:?}", block.index, &block);
        self.signers.borrow_mut().clear();
        self.truncate_db_from_block(block.index)?;
        self.add_block(block);
        Ok(())
    }

    pub fn get_sign_block(&self, keystore: &Option<Keystore>) -> Option<Block> {
        if self.get_height() < BLOCK_SIGNERS_START {
            trace!("Too early to start block signings");
            return None;
        }
        if keystore.is_none() {
            trace!("We can't sign blocks without keys");
            return None;
        }
        if self.get_height() < self.max_height() {
            trace!("No signing while syncing");
            return None;
        }

        let block = match self.last_full_block {
            None => { return None; }
            Some(ref block) => { block.clone() }
        };
        // TODO maybe make some config option to mine signing blocks above?
        let sign_count = self.get_height() - block.index;
        if sign_count >= BLOCK_SIGNERS_MIN {
            trace!("Block {} has enough signing blocks", block.index);
            return None;
        }
        if let Some(block) = &self.last_block {
            if block.timestamp + 60 > Utc::now().timestamp() {
                info!("Waiting for other blocks before signing.");
                return None;
            }
        }
        let (last_hash, last_index) = match &self.last_block {
            Some(block) => (block.hash.clone(), block.index),
            None => { return None; }
        };

        let keystore = keystore.clone().unwrap().clone();
        let signers: HashSet<Bytes> = self.get_block_signers(&block).into_iter().collect();
        if signers.contains(&keystore.get_public()) {
            for index in block.index..=self.get_height() {
                let b = self.get_block(index).unwrap();
                if b.pub_key == keystore.get_public() {
                    info!("We already mined signing block for block {}", block.index);
                    return None;
                }
            }

            info!("We have an honor to mine signing block!");
            let mut block = Block::new(None, Bytes::default(), last_hash, SIGNER_DIFFICULTY);
            block.index = last_index + 1;
            return Some(block);
        } else if !signers.is_empty() {
            info!("Signing block must be mined by other nodes");
        }
        None
    }

    pub fn update_sign_block_for_mining(&self, mut block: Block) -> Option<Block> {
        if let Some(full_block) = &self.last_full_block {
            let sign_count = self.get_height() - full_block.index;
            if sign_count >= BLOCK_SIGNERS_MIN {
                return None;
            }
            if let Some(last) = &self.last_block {
                block.index = last.index + 1;
                block.prev_block_hash = last.hash.clone();
                return Some(block);
            }
        }
        None
    }

    pub fn is_waiting_signers(&self) -> bool {
        if let Some(full_block) = &self.last_full_block {
            let sign_count = self.get_height() - full_block.index;
            if sign_count < BLOCK_SIGNERS_MIN {
                return true;
            }
        }

        false
    }

    /// Adds block to blocks table
    fn add_block_to_table(&mut self, block: Block) -> sqlite::Result<State> {
        let mut statement = self.db.prepare(SQL_ADD_BLOCK)?;
        statement.bind(1, block.index as i64)?;
        statement.bind(2, block.timestamp as i64)?;
        statement.bind(3, block.version as i64)?;
        statement.bind(4, block.difficulty as i64)?;
        statement.bind(5, block.random as i64)?;
        statement.bind(6, block.nonce as i64)?;
        match &block.transaction {
            None => { statement.bind(7, "")?; }
            Some(transaction) => {
                statement.bind(7, transaction.to_string().as_str())?;
            }
        }
        statement.bind(8, &**block.prev_block_hash)?;
        statement.bind(9, &**block.hash)?;
        statement.bind(10, &**block.pub_key)?;
        statement.bind(11, &**block.signature)?;
        statement.next()
    }

    /// Adds transaction to transactions table
    fn add_transaction_to_table(&mut self, index: u64, timestamp: i64, t: &Transaction) -> sqlite::Result<State> {
        let sql = match t.class.as_ref() {
            "domain" => SQL_ADD_DOMAIN,
            "zone" => SQL_ADD_ZONE,
            _ => return Err(sqlite::Error { code: None, message: None })
        };

        let mut statement = self.db.prepare(sql)?;
        statement.bind(1, index as i64)?;
        statement.bind(2, timestamp)?;
        statement.bind(3, &**t.identity)?;
        statement.bind(4, &**t.confirmation)?;
        statement.bind(5, t.data.as_ref() as &str)?;
        statement.bind(6, &**t.pub_key)?;
        statement.next()
    }

    pub fn get_block(&self, index: u64) -> Option<Block> {
        match self.db.prepare(SQL_GET_BLOCK_BY_ID) {
            Ok(mut statement) => {
                statement.bind(1, index as i64).expect("Error in bind");
                while statement.next().unwrap() == State::Row {
                    return match Self::get_block_from_statement(&mut statement) {
                        None => {
                            error!("Something wrong with block in DB!");
                            None
                        }
                        Some(block) => {
                            //trace!("Loaded block: {:?}", &block);
                            Some(block)
                        }
                    };
                }
                None
            }
            Err(_) => {
                warn!("Can't find requested block {}", index);
                None
            }
        }
    }

    /// Gets last block that has a Transaction within
    pub fn get_last_full_block(&self, before: u64, pub_key: Option<&[u8]>) -> Option<Block> {
        if let Some(block) = &self.last_full_block {
            if block.index < before {
                match pub_key {
                    None => { return Some(block.clone()); }
                    Some(key) => {
                        if block.pub_key.deref().eq(key) {
                            return Some(block.clone());
                        }
                    }
                }
            }
        }

        let mut statement = match pub_key {
            None => {
                let mut statement = self.db.prepare(SQL_GET_LAST_FULL_BLOCK).expect("Unable to prepare");
                statement.bind(1, before as i64).expect("Unable to bind");
                statement
            }
            Some(pub_key) => {
                let mut statement = self.db.prepare(SQL_GET_LAST_FULL_BLOCK_FOR_KEY).expect("Unable to prepare");
                statement.bind(1, before as i64).expect("Unable to bind");
                statement.bind(2, pub_key).expect("Unable to bind");
                statement
            }
        };
        while statement.next().unwrap() == State::Row {
            return match Self::get_block_from_statement(&mut statement) {
                None => {
                    error!("Something wrong with block in DB!");
                    None
                }
                Some(block) => {
                    //trace!("Got last full block: {:?}", &block);
                    Some(block)
                }
            };
        }
        None
    }

    /// Checks if any domain is available to mine for this client (pub_key)
    pub fn is_domain_available(&self, height: u64, domain: &str, keystore: &Keystore) -> bool {
        if domain.is_empty() {
            return false;
        }
        let identity_hash = hash_identity(domain, None);
        if !self.is_id_available(height, &identity_hash, &keystore.get_public(), false) {
            return false;
        }

        let parts: Vec<&str> = domain.rsplitn(2, ".").collect();
        if parts.len() > 1 {
            // We do not support third level domains
            if parts.last().unwrap().contains(".") {
                return false;
            }
            return self.is_zone_in_blockchain(height, parts.first().unwrap());
        }
        true
    }

    /// Checks if this identity is free or is owned by the same pub_key
    pub fn is_id_available(&self, height: u64, identity: &Bytes, public_key: &Bytes, zone: bool) -> bool {
        let sql = match zone {
            true => { SQL_GET_ZONE_PUBLIC_KEY_BY_ID }
            false => { SQL_GET_DOMAIN_PUBLIC_KEY_BY_ID }
        };

        let mut statement = self.db.prepare(sql).unwrap();
        statement.bind(1, height as i64).expect("Error in bind");
        statement.bind(2, &***identity).expect("Error in bind");
        while let State::Row = statement.next().unwrap() {
            let pub_key = Bytes::from_bytes(&statement.read::<Vec<u8>>(0).unwrap());
            if !pub_key.eq(public_key) {
                return false;
            }
        }
        true
    }

    pub fn get_zones(&self) -> Vec<ZoneData> {
        let mut map = HashMap::new();
        match self.db.prepare(SQL_GET_ZONES) {
            Ok(mut statement) => {
                while statement.next().unwrap() == State::Row {
                    let data = statement.read::<String>(0).unwrap();
                    //debug!("Got zone data {}", &data);
                    if let Ok(zone_data) = serde_json::from_str::<ZoneData>(&data) {
                        map.insert(zone_data.name.clone(), zone_data);
                    }
                }
            }
            Err(e) => {
                warn!("Can't get zones from DB {}", e);
            }
        }
        let result: Vec<ZoneData> = map.drain().map(|(_, value)| value).collect();
        result
    }

    /// Checks if some zone exists in our blockchain
    pub fn is_zone_in_blockchain(&self, height: u64, zone: &str) -> bool {
        if self.zones.borrow().contains(zone) {
            return true;
        }

        // Checking for existing zone in DB
        let identity_hash = hash_identity(zone, None);
        if self.is_id_in_blockchain(height, &identity_hash, true) {
            // If there is such a zone
            self.zones.borrow_mut().insert(zone.to_owned());
            return true;
        }
        false
    }

    /// Checks if some id exists in our blockchain
    pub fn is_id_in_blockchain(&self, height: u64, id: &Bytes, zone: bool) -> bool {
        let sql = match zone {
            true => { SQL_GET_ZONE_PUBLIC_KEY_BY_ID }
            false => { SQL_GET_DOMAIN_PUBLIC_KEY_BY_ID }
        };
        // Checking for existing zone in DB
        let mut statement = self.db.prepare(sql).unwrap();
        statement.bind(1, height as i64).expect("Error in bind");
        statement.bind(2, &***id).expect("Error in bind");
        while let State::Row = statement.next().unwrap() {
            // If there is such a zone
            return true;
        }
        false
    }

    pub fn can_mine_domain(&self, height: u64, domain: &str, pub_key: &Bytes) -> MineResult {
        let name = domain.to_lowercase();
        if !check_domain(&name, true) {
            return WrongName;
        }
        let zone = get_domain_zone(&name);
        if !self.is_zone_in_blockchain(height, &zone) {
            return WrongZone;
        }
        if let Some(transaction) = self.get_domain_transaction(&name) {
            if transaction.pub_key.ne(pub_key) {
                return NotOwned;
            }
        }
        let identity_hash = hash_identity(&name, None);
        if let Some(last) = self.get_last_full_block(MAX, Some(&pub_key)) {
            let new_id = !self.is_id_in_blockchain(height, &identity_hash, false);
            let time = last.timestamp + NEW_DOMAINS_INTERVAL - Utc::now().timestamp();
            if new_id && time > 0 {
                return Cooldown { time }
            }
        }

        Fine
    }

    /// Gets full Transaction info for any domain. Used by DNS part.
    pub fn get_domain_transaction(&self, domain: &str) -> Option<Transaction> {
        if domain.is_empty() {
            return None;
        }
        let identity_hash = hash_identity(domain, None);

        let mut statement = self.db.prepare(SQL_GET_DOMAIN_BY_ID).unwrap();
        statement.bind(1, &**identity_hash).expect("Error in bind");
        while let State::Row = statement.next().unwrap() {
            let timestamp = statement.read::<i64>(1).unwrap();
            if timestamp < Utc::now().timestamp() - DOMAIN_LIFETIME {
                // This domain is too old
                return None;
            }
            let identity = Bytes::from_bytes(&statement.read::<Vec<u8>>(2).unwrap());
            let confirmation = Bytes::from_bytes(&statement.read::<Vec<u8>>(3).unwrap());
            let class = String::from("domain");
            let data = statement.read::<String>(4).unwrap();
            let pub_key = Bytes::from_bytes(&statement.read::<Vec<u8>>(5).unwrap());
            let transaction = Transaction { identity, confirmation, class, data, pub_key };
            debug!("Found transaction for domain {}: {:?}", domain, &transaction);
            if transaction.check_identity(domain) {
                return Some(transaction);
            }
        }
        None
    }

    pub fn get_domain_info(&self, domain: &str) -> Option<String> {
        match self.get_domain_transaction(domain) {
            None => { None }
            Some(transaction) => { Some(transaction.data) }
        }
    }

    pub fn get_my_domains(&self, keystore: &Option<Keystore>) -> HashMap<Bytes, (String, i64, DomainData)> {
        if keystore.is_none() {
            return HashMap::new();
        }

        let mut result = HashMap::new();
        let keystore = keystore.clone().unwrap();
        let pub_key = keystore.get_public();
        let mut statement = self.db.prepare(SQL_GET_DOMAINS_BY_KEY).unwrap();
        statement.bind(1, &**pub_key).expect("Error in bind");
        while let State::Row = statement.next().unwrap() {
            let index = statement.read::<i64>(0).unwrap() as u64;
            let timestamp = statement.read::<i64>(1).unwrap();
            let identity = Bytes::from_bytes(&statement.read::<Vec<u8>>(2).unwrap());
            let confirmation = Bytes::from_bytes(&statement.read::<Vec<u8>>(3).unwrap());
            let class = String::from("domain");
            let data = statement.read::<String>(4).unwrap();
            let pub_key = Bytes::from_bytes(&statement.read::<Vec<u8>>(5).unwrap());
            let transaction = Transaction { identity: identity.clone(), confirmation: confirmation.clone(), class, data, pub_key };
            //debug!("Found transaction for domain {}: {:?}", domain, &transaction);
            if let Some(data) = transaction.get_domain_data() {
                let mut domain = keystore.decrypt(data.domain.as_slice(), &confirmation.as_slice()[..12]);
                if domain.is_empty() {
                    // Legacy encryption scheme
                    for i in 1..=10 {
                        let b = self.get_block(index - i).unwrap();
                        domain = keystore.decrypt(data.domain.as_slice(), &b.hash.as_slice()[..12]);
                        if !domain.is_empty() {
                            break;
                        }
                    }
                }

                let mut domain = String::from_utf8(domain.to_vec()).unwrap();
                if domain.is_empty() {
                    domain = String::from("unknown");
                }
                trace!("Found my domain {}", domain);
                result.insert(identity, (domain, timestamp, data));
            }
        }
        result
    }

    pub fn get_zone_difficulty(&self, zone: &str) -> u32 {
        let zones = self.get_zones();
        for z in zones.iter() {
            if z.name.eq(zone) {
                return z.difficulty;
            }
        }
        u32::MAX
    }

    pub fn last_block(&self) -> Option<Block> {
        self.last_block.clone()
    }

    pub fn get_height(&self) -> u64 {
        match self.last_block {
            None => { 0u64 }
            Some(ref block) => {
                block.index
            }
        }
    }

    pub fn get_last_hash(&self) -> Bytes {
        match &self.last_block {
            None => { Bytes::default() }
            Some(block) => { block.hash.clone() }
        }
    }

    pub fn next_allowed_full_block(&self) -> u64 {
        match self.last_full_block {
            None => { self.get_height() + 1 }
            Some(ref block) => {
                if block.index < BLOCK_SIGNERS_START {
                    self.get_height() + 1
                } else {
                    max(block.index + BLOCK_SIGNERS_MIN, self.get_height() + 1)
                }
            }
        }
    }

    pub fn max_height(&self) -> u64 {
        self.max_height
    }

    pub fn update_max_height(&mut self, height: u64) {
        self.max_height = height;
    }

    pub fn check_new_block(&self, block: &Block) -> BlockQuality {
        self.check_block(block, &self.last_block, &self.last_full_block)
    }

    /// Check if this block can be added to our blockchain
    pub fn check_block(&self, block: &Block, last_block: &Option<Block>, last_full_block: &Option<Block>) -> BlockQuality {
        if block.version > CHAIN_VERSION {
            warn!("Ignoring block from unsupported version:\n{:?}", &block);
            return Bad;
        }
        let timestamp = Utc::now().timestamp();
        if block.timestamp > timestamp + 60 {
            warn!("Ignoring block from the future:\n{:?}", &block);
            return Bad;
        }
        if let Some(last) = last_block {
            if block.index > last.index + 1 {
                info!("Ignoring future block:\n{:?}", &block);
                return Future;
            }
        }
        if !check_public_key_strength(&block.pub_key, KEYSTORE_DIFFICULTY) {
            warn!("Ignoring block with weak public key:\n{:?}", &block);
            return Bad;
        }
        let difficulty = match &block.transaction {
            None => {
                if block.index == 1 {
                    ZONE_DIFFICULTY
                } else {
                    SIGNER_DIFFICULTY
                }
            }
            Some(t) => { self.get_difficulty_for_transaction(&t) }
        };
        if block.difficulty < difficulty {
            warn!("Block difficulty is lower than needed");
            return Bad;
        }
        if hash_difficulty(&block.hash) < block.difficulty {
            warn!("Ignoring block with low difficulty:\n{:?}", &block);
            return Bad;
        }
        if !check_block_hash(block) {
            warn!("Block {:?} has wrong hash! Ignoring!", &block);
            return Bad;
        }
        if !check_block_signature(&block) {
            warn!("Block {:?} has wrong signature! Ignoring!", &block);
            return Bad;
        }
        if let Some(prev_block) = self.get_block(block.index - 1) {
            if block.prev_block_hash.ne(&prev_block.hash) {
                warn!("Ignoring block with wrong previous hash:\n{:?}", &block);
                return Rewind;
            }
        }

        if let Some(transaction) = &block.transaction {
            let current_height = match last_block {
                None => { 0 }
                Some(block) => { block.index }
            };
            // TODO check for zone transaction
            let is_domain_available = self.is_id_available(current_height, &transaction.identity, &block.pub_key, false);
            let is_zone_available = self.is_id_available(current_height, &transaction.identity, &block.pub_key, true);
            if !is_domain_available || !is_zone_available {
                warn!("Block {:?} is trying to spoof an identity!", &block);
                return Bad;
            }
            if let Some(last) = self.get_last_full_block(block.index, Some(&block.pub_key)) {
                if last.index < block.index {
                    let new_id = !self.is_id_in_blockchain(block.index, &transaction.identity, false);
                    if new_id && last.timestamp + NEW_DOMAINS_INTERVAL > block.timestamp {
                        warn!("Block {:?} is mined too early!", &block);
                        return Bad;
                    }
                }
            }
            // Check if yggdrasil only property of zone is not violated
            if let Some(block_data) = transaction.get_domain_data() {
                let zones = self.get_zones();
                for z in &zones {
                    if z.name == block_data.zone {
                        if z.yggdrasil {
                            for record in &block_data.records {
                                if !is_yggdrasil_record(record) {
                                    warn!("Someone mined domain with clearnet records for Yggdrasil only zone!");
                                    return Bad;
                                }
                            }
                        }
                    }
                }
            }
        }
        match last_block {
            None => {
                if !block.is_genesis() {
                    warn!("Block is from the future, how is this possible?");
                    return Future;
                }
                if !self.origin.is_zero() && block.hash.ne(&self.origin) {
                    warn!("Mining gave us a bad block:\n{:?}", &block);
                    return Bad;
                }
            }
            Some(last_block) => {
                if block.timestamp < last_block.timestamp && block.index > last_block.index {
                    warn!("Ignoring block with timestamp/index collision:\n{:?}", &block);
                    return Bad;
                }
                if last_block.index + 1 < block.index {
                    warn!("Block {} arrived too early.", block.index);
                    return Future;
                }
                if block.index > BLOCK_SIGNERS_START {
                    // If this block is main, signed part of blockchain
                    if !self.is_good_sign_block(&block, last_full_block) {
                        return Bad;
                    }
                }

                if block.index <= last_block.index {
                    if block.index == last_block.index && last_block.hash == block.hash {
                        debug!("Ignoring block {}, we already have it", block.index);
                        return Twin;
                    }
                    if let Some(my_block) = self.get_block(block.index) {
                        return if my_block.hash.ne(&block.hash) {
                            warn!("Got forked block {} with hash {:?} instead of {:?}", block.index, block.hash, last_block.hash);
                            Fork
                        } else {
                            debug!("Ignoring block {}, we already have it", block.index);
                            Twin
                        };
                    }
                } else if block.prev_block_hash.ne(&last_block.hash) {
                    warn!("Ignoring block with wrong previous hash:\n{:?}", &block);
                    return Bad;
                }
            }
        }

        Good
    }

    /// Checks if this block is a good signature block
    fn is_good_sign_block(&self, block: &Block, last_full_block: &Option<Block>) -> bool {
        // If this is not a signing block
        if block.transaction.is_some() {
            return true;
        }
        if let Some(full_block) = &last_full_block {
            let sign_count = self.get_height() - full_block.index;
            if sign_count < BLOCK_SIGNERS_MIN {
                // Last full block is not locked enough
                if block.index > full_block.index && block.transaction.is_some() {
                    warn!("Not enough signing blocks over full {} block!", full_block.index);
                    return false;
                } else {
                    if !self.is_good_signer_for_block(&block, full_block) {
                        return false;
                    }
                }
            } else if sign_count < BLOCK_SIGNERS_ALL && block.transaction.is_none() {
                if !self.is_good_signer_for_block(&block, full_block) {
                    return false;
                }
            }
        }
        true
    }

    /// Check if this block's owner is a good candidate to sign last full block
    fn is_good_signer_for_block(&self, block: &Block, full_block: &Block) -> bool {
        // If we got a signing block
        let signers: HashSet<Bytes> = self.get_block_signers(full_block).into_iter().collect();
        if !signers.contains(&block.pub_key) {
            warn!("Ignoring block {} from '{:?}', as wrong signer!", block.index, &block.pub_key);
            return false;
        }
        // If this signers' public key has already locked/signed that block we return error
        for i in (full_block.index + 1)..block.index {
            let signer = self.get_block(i).expect("Error in DB!");
            if signer.pub_key == block.pub_key {
                warn!("Ignoring block {} from '{:?}', already signed by this key", block.index, &block.pub_key);
                return false;
            }
        }
        true
    }

    fn get_difficulty_for_transaction(&self, transaction: &Transaction) -> u32 {
        match transaction.class.as_ref() {
            "domain" => {
                return match serde_json::from_str::<DomainData>(&transaction.data) {
                    Ok(data) => {
                        for zone in self.get_zones().iter() {
                            if zone.name == data.zone {
                                return zone.difficulty;
                            }
                        }
                        u32::MAX
                    }
                    Err(_) => {
                        warn!("Error parsing DomainData from {:?}", transaction);
                        u32::MAX
                    }
                }
            }
            "zone" => { ZONE_DIFFICULTY }
            _ => { u32::MAX }
        }
    }

    /// Gets public keys of a node that needs to mine "signature" block above this block
    /// block - last full block
    pub fn get_block_signers(&self, block: &Block) -> Vec<Bytes> {
        let mut result = Vec::new();
        if block.index < BLOCK_SIGNERS_START || self.get_height() < block.index {
            return result;
        }

        assert!(block.transaction.is_some());
        if self.signers.borrow().has_signers_for(block.index) {
            return self.signers.borrow().signers.clone();
        }

        let mut set = HashSet::new();
        let tail = block.signature.get_tail_u64();
        let mut count = 1;
        let window = block.index - 1; // Without the last block
        while set.len() < BLOCK_SIGNERS_ALL as usize {
            let index = (tail.wrapping_mul(count) % window) + 1; // We want it to start from 1
            if let Some(b) = self.get_block(index) {
                if b.pub_key != block.pub_key && !set.contains(&b.pub_key) {
                    result.push(b.pub_key.clone());
                    set.insert(b.pub_key);
                }
            }
            count += 1;
        }
        trace!("Got signers for block {}: {:?}", block.index, &result);
        let mut signers = self.signers.borrow_mut();
        signers.index = block.index;
        signers.signers = result.clone();
        result
    }

    fn get_block_from_statement(statement: &mut Statement) -> Option<Block> {
        let index = statement.read::<i64>(0).unwrap() as u64;
        let timestamp = statement.read::<i64>(1).unwrap();
        let version = statement.read::<i64>(2).unwrap() as u32;
        let difficulty = statement.read::<i64>(3).unwrap() as u32;
        let random = statement.read::<i64>(4).unwrap() as u32;
        let nonce = statement.read::<i64>(5).unwrap() as u64;
        let transaction = Transaction::from_json(&statement.read::<String>(6).unwrap());
        let prev_block_hash = Bytes::from_bytes(statement.read::<Vec<u8>>(7).unwrap().as_slice());
        let hash = Bytes::from_bytes(statement.read::<Vec<u8>>(8).unwrap().as_slice());
        let pub_key = Bytes::from_bytes(statement.read::<Vec<u8>>(9).unwrap().as_slice());
        let signature = Bytes::from_bytes(statement.read::<Vec<u8>>(10).unwrap().as_slice());
        Some(Block::from_all_params(index, timestamp, version, difficulty, random, nonce, prev_block_hash, hash, pub_key, signature, transaction))
    }
}

struct SignersCache {
    index: u64,
    signers: Vec<Bytes>
}

impl SignersCache {
    pub fn new() -> RefCell<SignersCache> {
        let cache = SignersCache { index: 0, signers: Vec::new() };
        RefCell::new(cache)
    }

    pub fn has_signers_for(&self, index: u64) -> bool {
        self.index == index && !self.signers.is_empty()
    }

    pub fn clear(&mut self) {
        self.index = 0;
        self.signers.clear();
    }
}

#[cfg(test)]
pub mod tests {
    use crate::{Chain, Settings};
    use simplelog::{ConfigBuilder, TermLogger, TerminalMode, ColorChoice};
    use log::LevelFilter;

    fn init_logger() {
        let config = ConfigBuilder::new()
            .add_filter_ignore_str("mio::poll")
            .set_thread_level(LevelFilter::Off)
            .set_location_level(LevelFilter::Off)
            .set_target_level(LevelFilter::Error)
            .set_time_level(LevelFilter::Error)
            .set_time_to_local(true)
            .build();
        if let Err(e) = TermLogger::init(LevelFilter::Trace, config, TerminalMode::Stdout, ColorChoice::Auto) {
            println!("Unable to initialize logger!\n{}", e);
        }
    }

    #[test]
    pub fn load_and_check() {
        init_logger();
        let settings = Settings::default();
        let mut chain = Chain::new(&settings, "./tests/guachain.db");
        chain.check_chain(u64::MAX);
        assert_eq!(chain.get_height(), 214);
    }
}
