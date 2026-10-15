//! Glacier 1 audio bank reader (`.WHD` / `.WAV` and `streams.wav`).
//!
//! Decodes IMA ADPCM audio streams into standard 16-bit PCM WAV containers
//! and extracts raw Ogg Vorbis music streams for native Bevy audio playback.

use std::{collections::HashMap, fs::File, io::{Read, Seek, SeekFrom}, path::Path};

const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17,
    19, 21, 23, 25, 28, 31, 34, 37, 41, 45,
    50, 55, 60, 66, 73, 80, 88, 97, 107, 118,
    130, 143, 157, 173, 190, 209, 230, 253, 279, 307,
    337, 371, 408, 449, 494, 544, 598, 658, 724, 796,
    876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066,
    2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358,
    5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899,
    15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

const INDEX_TABLE: [i32; 16] = [
    -1, -1, -1, -1, 2, 4, 6, 8,
    -1, -1, -1, -1, 2, 4, 6, 8,
];

#[derive(Clone, Debug)]
pub struct AudioEntry {
    pub name: String,
    pub format_tag: u32,
    pub sample_rate: u32,
    pub bits_per_sample: u32,
    pub uncompressed_size: u32,
    pub compressed_size: u32,
    pub channels: u32,
    pub data_offset: u32,
    pub block_align: u32,
}

pub struct AudioArchive {
    pub entries: HashMap<String, AudioEntry>,
    wav_path: std::path::PathBuf,
    streams_path: std::path::PathBuf,
}

impl AudioArchive {
    pub fn load(whd_path: &Path, wav_path: &Path, streams_path: &Path) -> anyhow::Result<Self> {
        let whd = std::fs::read(whd_path)?;
        let mut entries = HashMap::new();
        let mut pos = 0x10;

        let total_len = if whd.len() > 8 {
            u32::from_le_bytes(whd[0..4].try_into().unwrap_or([0; 4])) as usize
        } else {
            whd.len()
        };

        while pos + 48 < total_len && pos < whd.len() {
            let Some(end) = whd[pos..].iter().position(|&b| b == 0) else { break };
            let name_bytes = &whd[pos..pos + end];
            let name = String::from_utf8_lossy(name_bytes).to_string();
            let meta_start = (pos + end + 4) & !3;
            if meta_start + 48 > whd.len() {
                break;
            }

            let read_u32 = |off: usize| {
                u32::from_le_bytes(whd[meta_start + off..meta_start + off + 4].try_into().unwrap_or([0; 4]))
            };

            let entry = AudioEntry {
                name: name.clone(),
                format_tag: read_u32(8),
                sample_rate: read_u32(12),
                bits_per_sample: read_u32(16),
                uncompressed_size: read_u32(20),
                compressed_size: read_u32(24),
                channels: read_u32(28),
                data_offset: read_u32(32),
                block_align: read_u32(40),
            };

            entries.insert(name, entry);
            pos = meta_start + 48;
        }

        Ok(Self {
            entries,
            wav_path: wav_path.to_path_buf(),
            streams_path: streams_path.to_path_buf(),
        })
    }

    /// Read and decode a sound entry into standard WAV (RIFF PCM 16-bit) or OGG Vorbis bytes.
    pub fn decode(&self, sound_name: &str) -> Option<Vec<u8>> {
        let entry = self.entries.get(sound_name)?;
        let in_streams = (entry.format_tag & 0x8000_0000) != 0 || entry.data_offset > 5_000_000;
        let path = if in_streams { &self.streams_path } else { &self.wav_path };

        let mut file = File::open(path).ok()?;
        file.seek(SeekFrom::Start(entry.data_offset as u64)).ok()?;
        let mut raw = vec![0u8; entry.compressed_size as usize];
        file.read_exact(&mut raw).ok()?;

        // If it's an Ogg Vorbis stream (starts with OggS)
        if raw.starts_with(b"OggS") {
            return Some(raw);
        }

        // Decode IMA ADPCM
        let block_align = if entry.block_align == 0 { 512 } else { entry.block_align as usize };
        let pcm = decode_ima_adpcm_mono(&raw, block_align);

        // Build RIFF WAVE header for PCM 16-bit
        let mut wav = Vec::with_capacity(44 + pcm.len());
        let data_size = pcm.len() as u32;
        let file_size = 36 + data_size;
        let sample_rate = entry.sample_rate.max(11025);
        let channels: u16 = 1;
        let bits_per_sample: u16 = 16;
        let byte_rate = sample_rate * channels as u32 * (bits_per_sample as u32 / 8);
        let block_align_pcm = channels * (bits_per_sample / 8);

        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&file_size.to_le_bytes());
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes()); // subchunk1 size (16 for PCM)
        wav.extend_from_slice(&1u16.to_le_bytes());  // audio format (1 = PCM)
        wav.extend_from_slice(&channels.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&byte_rate.to_le_bytes());
        wav.extend_from_slice(&block_align_pcm.to_le_bytes());
        wav.extend_from_slice(&bits_per_sample.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        wav.extend_from_slice(&pcm);

        Some(wav)
    }
}

fn decode_ima_adpcm_mono(data: &[u8], block_align: usize) -> Vec<u8> {
    let mut out: Vec<i16> = Vec::with_capacity(data.len() * 2);
    let mut offset = 0;

    while offset < data.len() {
        let block_end = (offset + block_align).min(data.len());
        let block = &data[offset..block_end];
        if block.len() < 4 {
            break;
        }

        let mut sample = i16::from_le_bytes([block[0], block[1]]) as i32;
        let mut index = (block[2] as usize).min(88);
        out.push(sample as i16);

        for &byte in &block[4..] {
            for &nibble in &[byte & 0x0f, (byte >> 4) & 0x0f] {
                let n = nibble as usize;
                let step = STEP_TABLE[index];
                let mut diff = step >> 3;
                if (n & 1) != 0 { diff += step >> 2; }
                if (n & 2) != 0 { diff += step >> 1; }
                if (n & 4) != 0 { diff += step; }

                if (n & 8) != 0 {
                    sample -= diff;
                } else {
                    sample += diff;
                }
                sample = sample.clamp(-32768, 32767);
                index = ((index as i32 + INDEX_TABLE[n]).clamp(0, 88)) as usize;
                out.push(sample as i16);
            }
        }

        offset += block_align;
    }

    let mut bytes = Vec::with_capacity(out.len() * 2);
    for s in out {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    bytes
}
