//! `.LOC` localized strings: a tree of named nodes. Leaves are `key\0`, `u8 1`, `value\0`
//! (UTF-8). The tree also carries child counts and skip offsets, but every string the game
//! needs can be found by its key, so this reader just indexes the leaves.

use std::collections::HashMap;

#[derive(Default)]
pub struct Strings {
    map: HashMap<String, String>,
}

impl Strings {
    pub fn parse(d: &[u8]) -> Self {
        let mut map = HashMap::new();
        let mut i = 1;
        while i + 2 < d.len() {
            // A leaf value is introduced by "\0\x01" right after its key.
            if d[i] == 0 && d[i + 1] == 1 {
                let key_start = d[..i].iter().rposition(|&c| c < 0x20).map_or(0, |p| p + 1);
                let value_end = d[i + 2..].iter().position(|&c| c == 0).map_or(d.len(), |p| i + 2 + p);
                if let (Ok(k), Ok(v)) = (std::str::from_utf8(&d[key_start..i]), std::str::from_utf8(&d[i + 2..value_end])) {
                    if !k.is_empty() && k.chars().all(|c| c.is_ascii_graphic()) {
                        // First definition wins: level-specific text precedes shared fallbacks.
                        map.entry(k.to_owned()).or_insert_with(|| v.to_owned());
                    }
                }
                i = value_end + 1;
            } else {
                i += 1;
            }
        }
        Self { map }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}
