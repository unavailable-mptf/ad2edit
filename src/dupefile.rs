//! The AD2 file wrapper: "AD2F" + revision + \n + info block + \n + LZMA body.
//! Also the LZMA "alone" layer, which is where GMod and liblzma disagree.

use std::io::Read;

use liblzma::read::{XzDecoder, XzEncoder};
use liblzma::stream::{LzmaOptions, Stream};

// The 13-byte header on an LZMA "alone" (.lzma) stream:
//   byte 0     properties, packed as (pb * 5 + lp) * 9 + lc
//   bytes 1-4  dictionary size, u32 little-endian
//   bytes 5-12 uncompressed size, u64 little-endian
//              (all 0xFF means "unknown, look for an end-of-payload marker")
pub const ALONE_HEADER: usize = 13;

#[derive(Debug)]
pub struct AloneHeader {
    pub lc: u32,
    pub lp: u32,
    pub pb: u32,
    pub dict: u32,
    pub size: u64,
}

impl AloneHeader {
    pub fn parse(b: &[u8]) -> Option<AloneHeader> {
        if b.len() < ALONE_HEADER {
            return None;
        }
        // Unpack the properties byte by reversing the packing formula.
        let props = b[0] as u32;
        let lc = props % 9;
        let rest = props / 9;
        Some(AloneHeader {
            lc,
            lp: rest % 5,
            pb: rest / 5,
            dict: u32::from_le_bytes(b[1..5].try_into().unwrap()),
            size: u64::from_le_bytes(b[5..13].try_into().unwrap()),
        })
    }

    pub fn describe(&self) -> String {
        let size = if self.size == u64::MAX {
            "unknown (end-of-payload marker)".to_string()
        } else {
            format!("{}", self.size)
        };
        format!(
            "lc={} lp={} pb={} dict={} ({} KB) uncompressed size={}",
            self.lc,
            self.lp,
            self.pb,
            self.dict,
            self.dict / 1024,
            size
        )
    }
}

pub fn decompress(body: &[u8]) -> Result<Vec<u8>, String> {
    // new_lzma_decoder builds a stream for ALONE format. The plain
    // XzDecoder::new() expects .xz, a different container, and would reject
    // this outright.
    let stream = Stream::new_lzma_decoder(u64::MAX).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    XzDecoder::new_stream(body, stream)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Compress to alone format using explicit LZMA parameters, then optionally
/// overwrite the uncompressed-size field.
///
/// liblzma's alone encoder hardcodes that field to 0xFF * 8 and relies on an
/// end-of-payload marker instead. GMod's util.Compress writes a real length.
/// Rather than guess which one GMod's decompressor needs, we match whatever
/// the original file did.
pub fn compress(
    data: &[u8],
    lc: u32,
    lp: u32,
    pb: u32,
    dict: u32,
    write_real_size: bool,
) -> Result<Vec<u8>, String> {
    // Preset 6 sets the search strategy (match finder, nice length, depth).
    // Those affect compression ratio only, not the output's validity. The
    // four values below affect how the stream is decoded, so they must match.
    let mut opts = LzmaOptions::new_preset(6).map_err(|e| e.to_string())?;
    opts.dict_size(dict);
    opts.literal_context_bits(lc);
    opts.literal_position_bits(lp);
    opts.position_bits(pb);

    let stream = Stream::new_lzma_encoder(&opts).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    XzEncoder::new_stream(data, stream)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;

    if write_real_size && out.len() >= ALONE_HEADER {
        out[5..13].copy_from_slice(&(data.len() as u64).to_le_bytes());
    }
    Ok(out)
}

pub struct DupeFile {
    pub revision: u8,
    // Order-preserving: AD2 builds this with pairs(), so the order is
    // arbitrary but we keep whatever we read.
    pub info: Vec<(Vec<u8>, Vec<u8>)>,
    pub compressed: Vec<u8>,
}

pub fn parse(bytes: &[u8]) -> Result<DupeFile, String> {
    if bytes.len() < 6 || &bytes[0..4] != b"AD2F" {
        return Err("not an AD2 dupe; missing AD2F magic".into());
    }
    let revision = bytes[4];
    if bytes[5] != b'\n' {
        return Err(format!(
            "expected newline after revision, got {:#04x}",
            bytes[5]
        ));
    }

    let start = 6;
    let end = bytes[start..]
        .iter()
        .position(|&b| b == 0x02)
        .map(|o| start + o)
        .ok_or("no 0x02 info-block terminator found")?;

    // Layout is key \x01 value \x01 key \x01 value \x01 ... so splitting on
    // \x01 leaves a trailing empty piece, which chunks(2) discards.
    let parts: Vec<Vec<u8>> = bytes[start..end]
        .split(|&b| b == 0x01)
        .map(|c| c.to_vec())
        .collect();

    let mut info = Vec::new();
    for pair in parts.chunks(2) {
        if let [k, v] = pair {
            if !k.is_empty() {
                info.push((k.clone(), v.clone()));
            }
        }
    }

    // Skip the 0x02, then the \n after it.
    let mut body_start = end + 1;
    if bytes.get(body_start) == Some(&b'\n') {
        body_start += 1;
    }

    Ok(DupeFile {
        revision,
        info,
        compressed: bytes[body_start..].to_vec(),
    })
}

pub fn build(revision: u8, info: &[(Vec<u8>, Vec<u8>)], compressed: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"AD2F");
    out.push(revision);
    out.push(b'\n');
    for (k, v) in info {
        out.extend_from_slice(k);
        out.push(0x01);
        out.extend_from_slice(v);
        out.push(0x01);
    }
    out.push(0x02);
    out.push(b'\n');
    out.extend_from_slice(compressed);
    out
}
