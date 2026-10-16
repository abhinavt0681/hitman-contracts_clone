//! `.LOC` localized strings: a tree of named nodes. Leaves are `key\0`, `u8 1`, `value\0`
//! (UTF-8). The tree also carries child counts and skip offsets, but every string the game
//! needs can be found by its key, so this reader just indexes the leaves.
//!
//! Item descriptions sit one level deeper: `Item_Key_Master_01\0`, `u8 2`, `u32` size, then
//! `FreeText\0 \x01 1\0 \x01 "This is a master key, and should open most doors"\0` and
//! `Type\0 \x01 "Key"\0`. They're indexed by item name (`description`).

use std::collections::HashMap;

#[derive(Default)]
pub struct Strings {
    map: HashMap<String, String>,
    descriptions: HashMap<String, String>,
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
        // Item descriptions: `<item>\0\x02<u32>FreeText\0\x01<n>\0\x01<text>\0`.
        let tag = b"FreeText\0\x01";
        let mut descriptions = HashMap::new();
        let mut at = 0;
        while let Some(off) = d[at..].windows(tag.len()).position(|w| w == tag) {
            let j = at + off;
            at = j + tag.len();
            // The owning node: its name ends 6 bytes before (`\0`, `\x02`, `u32`).
            if j < 7 || d[j - 6] != 0 || d[j - 5] != 2 {
                continue;
            }
            let name_end = j - 6;
            let name_start = d[..name_end].iter().rposition(|&c| c < 0x20).map_or(0, |p| p + 1);
            let Some(n) = d[at..].iter().position(|&c| c == 0) else { continue };
            if d.get(at + n + 1) != Some(&1) {
                continue;
            }
            let text_start = at + n + 2;
            let text_end = d[text_start..].iter().position(|&c| c == 0).map_or(d.len(), |p| text_start + p);
            if let (Ok(k), Ok(v)) = (std::str::from_utf8(&d[name_start..name_end]), std::str::from_utf8(&d[text_start..text_end])) {
                descriptions.entry(k.to_owned()).or_insert_with(|| v.to_owned());
            }
        }
        Self { map, descriptions }
    }

    pub fn descriptions(&self) -> impl Iterator<Item = (&String, &String)> {
        self.descriptions.iter()
    }

    /// An item's description (its `FreeText`), by item name.
    pub fn description(&self, item: &str) -> Option<&str> {
        self.descriptions.get(item).map(String::as_str)
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
