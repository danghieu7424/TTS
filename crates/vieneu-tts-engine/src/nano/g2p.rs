/****
 * [ATOM]: g2p
 * Chức năng: Tokenizer - Ánh xạ ký tự âm vị (Phoneme) thành Token IDs từ vocab của model v3nano
 ****/
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Deserialize)]
struct Config {
    vocab: HashMap<String, i64>,
}

pub struct Tokenizer {
    pub vocab: HashMap<char, i64>,
}

impl Tokenizer {
    pub fn new(config_path: &Path) -> Result<Self> {
        let config_str = fs::read_to_string(config_path)?;
        let config: Config = serde_json::from_str(&config_str)?;

        let mut vocab = HashMap::new();
        for (k, v) in config.vocab {
            if k.chars().count() == 1 {
                vocab.insert(k.chars().next().unwrap(), v);
            }
        }

        // Add default tokens
        vocab.insert('0', 0); // padding

        Ok(Self { vocab })
    }

    pub fn encode(&self, phonemes: &str, bos_id: i64, eos_id: i64) -> Vec<i64> {
        let mut ids = vec![bos_id];
        for c in phonemes.chars() {
            if let Some(&id) = self.vocab.get(&c) {
                ids.push(id);
            }
        }
        ids.push(eos_id);
        ids
    }
}
