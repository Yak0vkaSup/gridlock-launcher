//! glb1 blobs (written by publish_build.py in the game repo): a file cut into fixed blocks behind a
//! table of per-block hashes. Updating a file means keeping every block the old copy already holds,
//! wherever it moved to, and downloading only the rest with Range requests. Measured on two builds
//! three days apart: 38 MB of the 2.2 GB .ucas actually differ.
//!
//! Blob layout, little endian:
//!   header  24 B  "GLB1" | block size u32 | file size u64 | block count u32 | reserved u32
//!   table   28 B per block: adler32 u32 | sha256[:16] | stored bytes u32 | flags u32 (bit 0 = zlib)
//!   payload the blocks in order, each `stored` bytes long

use anyhow::{bail, Context};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

pub const MAGIC: &[u8; 4] = b"GLB1";
pub const HEADER: usize = 24;
pub const ENTRY: usize = 28;
pub const FLAG_ZLIB: u32 = 1;
const ADLER_MOD: u32 = 65521;
/// Blocks we already have that sit between two missing ones are fetched again when the stretch is
/// this short: one Range request for the whole run beats several small ones.
const GAP: usize = 8;
const READ_CHUNK: usize = 4 << 20;

/// (bytes of the file accounted for, bytes that came over the network)
pub type Tick = Arc<dyn Fn(u64, u64) + Send + Sync>;

#[derive(Clone, Debug)]
pub struct Entry {
    pub weak: u32,
    pub strong: [u8; 16],
    pub stored: u32,
    pub flags: u32,
}

#[derive(Clone, Debug)]
pub struct Table {
    pub block: u32,
    pub size: u64,
    pub entries: Vec<Entry>,
}

pub fn nblocks(size: u64, block: u32) -> usize {
    size.div_ceil(block as u64) as usize
}

pub fn table_len(size: u64, block: u32) -> usize {
    HEADER + ENTRY * nblocks(size, block)
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes(b.try_into().unwrap())
}

impl Table {
    pub fn parse(bytes: &[u8]) -> anyhow::Result<Table> {
        if bytes.len() < HEADER || &bytes[..4] != MAGIC {
            bail!("not a GLB1 blob");
        }
        let block = le32(&bytes[4..8]);
        let size = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let n = le32(&bytes[16..20]) as usize;
        if block == 0 || n != nblocks(size, block) {
            bail!("blob header is inconsistent");
        }
        if bytes.len() < HEADER + ENTRY * n {
            bail!("blob table is short");
        }
        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let e = &bytes[HEADER + ENTRY * i..HEADER + ENTRY * (i + 1)];
            entries.push(Entry {
                weak: le32(&e[0..4]),
                strong: e[4..20].try_into().unwrap(),
                stored: le32(&e[20..24]),
                flags: le32(&e[24..28]),
            });
        }
        Ok(Table { block, size, entries })
    }

    /// Length of block `i` in the file (only the last one can be short).
    pub fn raw_len(&self, i: usize) -> usize {
        let start = i as u64 * self.block as u64;
        (self.size - start).min(self.block as u64) as usize
    }

    pub fn payload_start(&self) -> u64 {
        (HEADER + ENTRY * self.entries.len()) as u64
    }

    /// Offset of every block inside the payload, plus the payload end.
    pub fn payload_offsets(&self) -> Vec<u64> {
        let mut offs = Vec::with_capacity(self.entries.len() + 1);
        let mut at = 0u64;
        for e in &self.entries {
            offs.push(at);
            at += e.stored as u64;
        }
        offs.push(at);
        offs
    }
}

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    // 5552 = zlib's NMAX: the longest run whose sums still fit in u32 before reducing
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= ADLER_MOD;
        b %= ADLER_MOD;
    }
    (b << 16) | a
}

/// adler32 of a window that slides one byte at a time.
struct Rolling {
    a: u32,
    b: u32,
    n_mod: u32,
}

impl Rolling {
    fn new(window: &[u8]) -> Self {
        let s = adler32(window);
        Rolling { a: s & 0xffff, b: s >> 16, n_mod: window.len() as u32 % ADLER_MOD }
    }
    #[inline]
    fn roll(&mut self, out: u8, inp: u8) {
        self.a = (self.a + inp as u32 + ADLER_MOD - out as u32) % ADLER_MOD;
        self.b = (self.b + self.a + 2 * ADLER_MOD - (self.n_mod * out as u32) % ADLER_MOD - 1) % ADLER_MOD;
    }
    #[inline]
    fn value(&self) -> u32 {
        (self.b << 16) | self.a
    }
}

/// One bit per weak hash we are looking for: the rolling scan touches this per byte and the map
/// only on a hit (2^22 bits = 512 KB, stays in cache).
struct Filter(Vec<u64>);

impl Filter {
    const BITS: u32 = 22;
    fn new() -> Self {
        Filter(vec![0u64; 1 << (Self::BITS - 6)])
    }
    #[inline]
    fn slot(weak: u32) -> usize {
        (weak.wrapping_mul(0x9E37_79B1) >> (32 - Self::BITS)) as usize
    }
    fn set(&mut self, weak: u32) {
        let s = Self::slot(weak);
        self.0[s >> 6] |= 1 << (s & 63);
    }
    #[inline]
    fn has(&self, weak: u32) -> bool {
        let s = Self::slot(weak);
        self.0[s >> 6] & (1 << (s & 63)) != 0
    }
}

fn write_block(part: &mut File, table: &Table, i: usize, data: &[u8]) -> anyhow::Result<()> {
    part.seek(SeekFrom::Start(i as u64 * table.block as u64))?;
    part.write_all(data)?;
    Ok(())
}

/// A few sampled bytes of a window: with the weak hash, enough to recognise the same window again.
#[inline]
fn fingerprint(window: &[u8]) -> u64 {
    let n = window.len();
    let s = [window[0], window[n / 4], window[n / 2], window[n * 3 / 4], window[n - 1], window[n / 8], window[n * 7 / 8], window[n / 3]];
    u64::from_le_bytes(s)
}

/// Copies into `part` every full block of the new file that `old` contains anywhere (rsync's
/// rolling-checksum search, old file as the haystack). Returns the number of bytes reused.
pub fn reuse_from_old(
    old: &Path,
    table: &Table,
    found: &mut [bool],
    part: &mut File,
    mut on_block: impl FnMut(usize),
) -> anyhow::Result<u64> {
    let block = table.block as usize;
    let mut by_weak: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut filter = Filter::new();
    for (i, e) in table.entries.iter().enumerate() {
        if !found[i] && table.raw_len(i) == block {
            by_weak.entry(e.weak).or_default().push(i as u32);
            filter.set(e.weak);
        }
    }
    if by_weak.is_empty() {
        return Ok(0);
    }
    let mut file = File::open(old)?;
    let mut buf = vec![0u8; block + READ_CHUNK];
    let mut filled = 0usize;
    let mut pos = 0usize;
    let mut reused = 0u64;

    // moves the unread tail to the front and reads until the buffer is full or the file ends
    fn refill(file: &mut File, buf: &mut [u8], filled: &mut usize, pos: &mut usize) -> std::io::Result<()> {
        buf.copy_within(*pos..*filled, 0);
        *filled -= *pos;
        *pos = 0;
        while *filled < buf.len() {
            let n = file.read(&mut buf[*filled..])?;
            if n == 0 {
                break;
            }
            *filled += n;
        }
        Ok(())
    }

    refill(&mut file, &mut buf, &mut filled, &mut pos)?;
    if filled < block {
        return Ok(0);
    }
    let mut roll = Rolling::new(&buf[..block]);
    // windows whose weak hash matched a wanted block but whose content did not: repeated data (zero
    // padding, a repeated phrase) would otherwise cost one full hash per byte position
    let mut failed: std::collections::HashSet<(u32, u64)> = std::collections::HashSet::new();
    loop {
        // 1. the window at `pos`
        let w = roll.value();
        if filter.has(w) {
            if let Some(cands) = by_weak.get(&w) {
                let window = &buf[pos..pos + block];
                let fp = fingerprint(window);
                let strong: Option<[u8; 16]> = if failed.contains(&(w, fp)) {
                    None
                } else {
                    Some(Sha256::digest(window)[..16].try_into().unwrap())
                };
                let mut hit = false;
                if let Some(strong) = strong {
                    for &i in cands {
                        let i = i as usize;
                        if !found[i] && table.entries[i].strong == strong {
                            write_block(part, table, i, window)?;
                            found[i] = true;
                            reused += block as u64;
                            on_block(i);
                            hit = true;
                        }
                    }
                    if !hit {
                        if failed.len() > 65536 {
                            failed.clear();
                        }
                        failed.insert((w, fp));
                    } else if cands.iter().all(|&i| found[i as usize]) {
                        by_weak.remove(&w);
                    }
                }
                if hit {
                    pos += block;
                    if pos + block > filled {
                        refill(&mut file, &mut buf, &mut filled, &mut pos)?;
                        if filled < block {
                            break;
                        }
                    }
                    roll = Rolling::new(&buf[pos..pos + block]);
                    continue;
                }
            }
        }
        // 2. slide by one byte
        if pos + block >= filled {
            refill(&mut file, &mut buf, &mut filled, &mut pos)?;
            if pos + block >= filled {
                break;
            }
        }
        roll.roll(buf[pos], buf[pos + block]);
        pos += 1;
    }
    Ok(reused)
}

/// Marks the blocks a partially written `.part` already holds (their strong hash matches).
/// Returns the stored size of the blocks it found.
pub fn verify_part(part: &mut File, table: &Table, found: &mut [bool]) -> anyhow::Result<u64> {
    part.seek(SeekFrom::Start(0))?;
    let mut buf = vec![0u8; table.block as usize];
    let mut ok = 0u64;
    for i in 0..table.entries.len() {
        let len = table.raw_len(i);
        part.read_exact(&mut buf[..len])?;
        if !found[i] && Sha256::digest(&buf[..len])[..16] == table.entries[i].strong {
            found[i] = true;
            ok += table.entries[i].stored as u64;
        }
    }
    Ok(ok)
}

/// Stretches of missing blocks, inclusive; present blocks inside a gap of at most `gap` are absorbed.
pub fn missing_runs(found: &[bool], gap: usize) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, &have) in found.iter().enumerate() {
        if have {
            continue;
        }
        match runs.last_mut() {
            Some((_, last)) if i - *last <= gap + 1 => *last = i,
            _ => runs.push((i, i)),
        }
    }
    runs
}

pub async fn fetch_range(client: &reqwest::Client, url: &str, start: u64, end: u64) -> anyhow::Result<Vec<u8>> {
    if end <= start {
        return Ok(Vec::new());
    }
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={}-{}", start, end - 1))
        .send()
        .await?;
    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        bail!("{} instead of a partial response", resp.status());
    }
    Ok(resp.bytes().await?.to_vec())
}

fn decode(table: &Table, i: usize, stored: &[u8]) -> anyhow::Result<Vec<u8>> {
    let e = &table.entries[i];
    let raw = if e.flags & FLAG_ZLIB != 0 {
        let mut out = Vec::with_capacity(table.block as usize);
        flate2::read::ZlibDecoder::new(stored).read_to_end(&mut out)?;
        out
    } else {
        stored.to_vec()
    };
    if raw.len() != table.raw_len(i) || Sha256::digest(&raw)[..16] != e.strong {
        bail!("block {i} does not match its hash");
    }
    Ok(raw)
}

/// Downloads every block not yet in `found` (in coalesced Range requests), writes them into `part`.
pub async fn download_missing(
    client: &reqwest::Client,
    url: &str,
    table: &Table,
    found: &mut [bool],
    part: &mut File,
    tick: &Tick,
) -> anyhow::Result<u64> {
    let offs = table.payload_offsets();
    let base = table.payload_start();
    let mut downloaded = 0u64;
    for (first, last) in missing_runs(found, GAP) {
        let (start, end) = (base + offs[first], base + offs[last + 1]);
        let resp = client
            .get(url)
            .header(reqwest::header::RANGE, format!("bytes={}-{}", start, end - 1))
            .send()
            .await?;
        if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            bail!("{} instead of a partial response", resp.status());
        }
        let mut stream = resp.bytes_stream();
        let mut pending: Vec<u8> = Vec::new();
        let mut cursor = 0usize;
        let mut i = first;
        while i <= last {
            let need = table.entries[i].stored as usize;
            while pending.len() - cursor < need {
                let Some(chunk) = stream.next().await else {
                    bail!("blob stream ended early");
                };
                let chunk = chunk.context("blob stream")?;
                downloaded += chunk.len() as u64;
                tick(0, chunk.len() as u64);
                if cursor > 0 && cursor > pending.len() / 2 {
                    pending.drain(..cursor);
                    cursor = 0;
                }
                pending.extend_from_slice(&chunk);
            }
            let raw = decode(table, i, &pending[cursor..cursor + need])?;
            cursor += need;
            write_block(part, table, i, &raw)?;
            if !found[i] {
                found[i] = true;
                tick(need as u64, 0);
            }
            i += 1;
        }
    }
    Ok(downloaded)
}

pub fn sha256_of(path: &Path) -> anyhow::Result<String> {
    let mut fh = File::open(path)?;
    let mut h = Sha256::new();
    std::io::copy(&mut fh, &mut h)?;
    Ok(hex::encode(h.finalize()))
}

/// Brings `dest` to the file the blob at `url` describes: whatever the old `dest` and a leftover
/// `.part` already hold is kept, the rest is fetched. `block` is the manifest's block size (used to
/// size the first request; the header is trusted after that). `tick` gets (stored bytes settled,
/// bytes received): every block counts once with its stored size, so the sum ends at the blob's
/// payload size, whether it was reused or fetched.
pub async fn fetch_blob(
    client: &reqwest::Client,
    url: &str,
    size: u64,
    sha256: &str,
    block: u32,
    dest: &Path,
    tick: Tick,
) -> anyhow::Result<()> {
    let head = fetch_range(client, url, 0, table_len(size, block) as u64).await.context("blob table")?;
    let mut table_bytes = head;
    if table_bytes.len() >= HEADER && &table_bytes[..4] == MAGIC {
        let real_block = le32(&table_bytes[4..8]);
        let want = table_len(size, real_block);
        if want > table_bytes.len() {
            table_bytes.extend(fetch_range(client, url, table_bytes.len() as u64, want as u64).await?);
        }
        table_bytes.truncate(want);
    }
    let table = Arc::new(Table::parse(&table_bytes)?);
    if table.size != size {
        bail!("blob says {} bytes, manifest says {size}", table.size);
    }
    tick(0, table_bytes.len() as u64);

    let part_path = dest.with_extension(match dest.extension() {
        Some(ext) => format!("{}.part", ext.to_string_lossy()),
        None => "part".to_string(),
    });
    let mut found = vec![false; table.entries.len()];
    let resume = std::fs::metadata(&part_path).map(|m| m.len() == size).unwrap_or(false);
    let mut part = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(!resume).open(&part_path)?;
    part.set_len(size)?;

    // sources first: the interrupted .part, then the old copy of the file
    if resume {
        let t = table.clone();
        let mut f = found;
        let tk = tick.clone();
        let r = tokio::task::spawn_blocking(move || {
            let n = verify_part(&mut part, &t, &mut f);
            (part, f, n)
        })
        .await?;
        part = r.0;
        found = r.1;
        let n = r.2?;
        if n > 0 {
            tk(n, 0);
        }
    }
    if dest.is_file() {
        let t = table.clone();
        let old = dest.to_path_buf();
        let mut f = found;
        let tk = tick.clone();
        let r = tokio::task::spawn_blocking(move || {
            let n = reuse_from_old(&old, &t, &mut f, &mut part, |i| tk(t.entries[i].stored as u64, 0));
            (part, f, n)
        })
        .await?;
        part = r.0;
        found = r.1;
        r.2?;
    }

    download_missing(client, url, &table, &mut found, &mut part, &tick).await?;
    part.flush()?;
    drop(part);

    let got = tokio::task::spawn_blocking({
        let p = part_path.clone();
        move || sha256_of(&p)
    })
    .await??;
    if got != sha256 {
        let _ = std::fs::remove_file(&part_path);
        bail!("hash mismatch after assembling the file (got {}, want {})", &got[..12], &sha256[..12]);
    }
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    std::fs::rename(&part_path, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    fn xorshift(seed: u64, len: usize) -> Vec<u8> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                (s >> 24) as u8
            })
            .collect()
    }

    #[test]
    fn adler32_matches_zlib() {
        // values from python: zlib.adler32(...)
        assert_eq!(adler32(b""), 0x1);
        assert_eq!(adler32(b"Wikipedia"), 0x11e60398);
        let bytes: Vec<u8> = (0..256u32).map(|x| x as u8).collect::<Vec<_>>().repeat(600);
        assert_eq!(adler32(&bytes), 0x1925e577);
        assert_eq!(adler32(&vec![b'x'; 200000]), 0xc5914b73);
    }

    #[test]
    fn rolling_equals_direct() {
        let data = xorshift(7, 300_000);
        let w = 4096;
        let mut r = Rolling::new(&data[..w]);
        for pos in 0..data.len() - w {
            if pos < 3000 || pos % 997 == 0 {
                assert_eq!(r.value(), adler32(&data[pos..pos + w]), "at {pos}");
            }
            r.roll(data[pos], data[pos + w]);
        }
    }

    /// The publisher's format, in Rust, for the tests.
    pub fn make_blob(data: &[u8], block: usize) -> Vec<u8> {
        let n = data.len().div_ceil(block);
        let mut table = Vec::new();
        let mut payload = Vec::new();
        for chunk in data.chunks(block) {
            let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
            enc.write_all(chunk).unwrap();
            let z = enc.finish().unwrap();
            let (stored, flags): (&[u8], u32) = if z.len() * 100 < chunk.len() * 95 { (&z, FLAG_ZLIB) } else { (chunk, 0) };
            table.extend_from_slice(&adler32(chunk).to_le_bytes());
            table.extend_from_slice(&Sha256::digest(chunk)[..16]);
            table.extend_from_slice(&(stored.len() as u32).to_le_bytes());
            table.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(stored);
        }
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(block as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u64).to_le_bytes());
        out.extend_from_slice(&(n as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend(table);
        out.extend(payload);
        out
    }

    /// Serves `blobs` (path -> bytes) with Range support, like R2 does. Returns the base url.
    pub fn serve(blobs: HashMap<String, Vec<u8>>) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let Some(body) = blobs.get(req.url()) else {
                    let _ = req.respond(tiny_http::Response::empty(404));
                    continue;
                };
                let range = req.headers().iter().find(|h| h.field.equiv("Range")).map(|h| h.value.to_string());
                let resp = match range {
                    Some(r) => {
                        let r = r.trim_start_matches("bytes=");
                        let (a, b) = r.split_once('-').unwrap();
                        let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse().unwrap());
                        let b = b.min(body.len() - 1);
                        tiny_http::Response::from_data(body[a..=b].to_vec()).with_status_code(206).with_header(
                            tiny_http::Header::from_bytes("Content-Range", format!("bytes {a}-{b}/{}", body.len())).unwrap(),
                        )
                    }
                    None => tiny_http::Response::from_data(body.clone()),
                };
                let _ = req.respond(resp);
            }
        });
        format!("http://{addr}")
    }

    struct Counter(AtomicU64, AtomicU64);

    fn tick(c: &Arc<Counter>) -> Tick {
        let c = c.clone();
        Arc::new(move |done, net| {
            c.0.fetch_add(done, Ordering::Relaxed);
            c.1.fetch_add(net, Ordering::Relaxed);
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn update_reuses_shifted_blocks() {
        let block = 65536usize;
        let old = {
            let mut v = xorshift(1, 3_000_000);
            v.extend(b"the quick brown fox ".repeat(40_000)); // compressible tail
            v
        };
        // new = old with 100 KB inserted at 1 MB (everything after it shifts) and a byte flipped at 2.5 MB
        let mut new = old[..1_000_000].to_vec();
        new.extend(xorshift(2, 100_000));
        new.extend_from_slice(&old[1_000_000..]);
        new[2_500_000] ^= 0xff;
        let sha = hex::encode(Sha256::digest(&new));
        let blob = make_blob(&new, block);
        let blob_len = blob.len() as u64;
        let url = format!("{}/b", serve(HashMap::from([("/b".to_string(), blob)])));

        let dir = std::env::temp_dir().join(format!("gl-delta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("file.bin");
        std::fs::write(&dest, &old).unwrap();

        let c = Arc::new(Counter(AtomicU64::new(0), AtomicU64::new(0)));
        fetch_blob(&reqwest::Client::new(), &url, new.len() as u64, &sha, block as u32, &dest, tick(&c)).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), new);
        let net = c.1.load(Ordering::Relaxed);
        // 100 KB insert + 1 changed block + the short last block + the table: far less than the file
        assert!(net < 600_000, "downloaded {net} of {}", new.len());
        let payload = blob_len - table_len(new.len() as u64, block as u32) as u64;
        assert_eq!(c.0.load(Ordering::Relaxed), payload);

        // fresh install: nothing to reuse, everything comes over the wire once
        let fresh = dir.join("fresh.bin");
        let c2 = Arc::new(Counter(AtomicU64::new(0), AtomicU64::new(0)));
        fetch_blob(&reqwest::Client::new(), &url, new.len() as u64, &sha, block as u32, &fresh, tick(&c2)).await.unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), new);
        assert!(c2.1.load(Ordering::Relaxed) >= new.len() as u64 / 2);

        // resume: a .part with the first half already written and the rest zeros
        let resumed = dir.join("resumed.bin");
        let mut half = new[..1_500_000].to_vec();
        half.resize(new.len(), 0);
        std::fs::write(dir.join("resumed.bin.part"), &half).unwrap();
        let c3 = Arc::new(Counter(AtomicU64::new(0), AtomicU64::new(0)));
        fetch_blob(&reqwest::Client::new(), &url, new.len() as u64, &sha, block as u32, &resumed, tick(&c3)).await.unwrap();
        assert_eq!(std::fs::read(&resumed).unwrap(), new);
        assert!(c3.1.load(Ordering::Relaxed) < new.len() as u64 * 2 / 3);

        // a wrong hash in the manifest must not leave a bad file behind
        let bad = dir.join("bad.bin");
        let err = fetch_blob(&reqwest::Client::new(), &url, new.len() as u64, &"0".repeat(64), block as u32, &bad, tick(&c3)).await;
        assert!(err.is_err() && !bad.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn zero_padding_is_cheap_and_reused() {
        let block = 65536usize;
        let mut old = xorshift(5, 500_000);
        old.extend(std::iter::repeat(0u8).take(2_000_000));
        old.extend(xorshift(6, 300_000));
        let mut new = old.clone();
        new.splice(200_000..200_000, xorshift(9, 50_000)); // shift everything after 200 KB
        let sha = hex::encode(Sha256::digest(&new));
        let blob = make_blob(&new, block);
        let url = format!("{}/b", serve(HashMap::from([("/b".to_string(), blob)])));
        let dir = std::env::temp_dir().join(format!("gl-zero-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("padded.bin");
        std::fs::write(&dest, &old).unwrap();
        let c = Arc::new(Counter(AtomicU64::new(0), AtomicU64::new(0)));
        let t0 = std::time::Instant::now();
        fetch_blob(&reqwest::Client::new(), &url, new.len() as u64, &sha, block as u32, &dest, tick(&c)).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), new);
        assert!(t0.elapsed() < Duration::from_secs(20), "took {:?}", t0.elapsed());
        assert!(c.1.load(Ordering::Relaxed) < 300_000, "downloaded {}", c.1.load(Ordering::Relaxed));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Serves the files of a directory with Range support, streaming from disk.
    pub fn serve_dir(dir: std::path::PathBuf) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let path = dir.join(req.url().trim_start_matches('/'));
                let Ok(mut file) = File::open(&path) else {
                    let _ = req.respond(tiny_http::Response::empty(404));
                    continue;
                };
                let len = file.metadata().unwrap().len();
                let range = req.headers().iter().find(|h| h.field.equiv("Range")).map(|h| h.value.to_string());
                match range {
                    Some(r) => {
                        let (a, b) = r.trim_start_matches("bytes=").split_once('-').unwrap();
                        let (a, b): (u64, u64) = (a.parse().unwrap(), b.parse().unwrap());
                        let b = b.min(len.saturating_sub(1));
                        file.seek(SeekFrom::Start(a)).unwrap();
                        let n = b + 1 - a;
                        let resp = tiny_http::Response::new(
                            tiny_http::StatusCode(206),
                            vec![tiny_http::Header::from_bytes("Content-Range", format!("bytes {a}-{b}/{len}")).unwrap()],
                            file.take(n),
                            Some(n as usize),
                            None,
                        );
                        let _ = req.respond(resp);
                    }
                    None => {
                        let _ = req.respond(tiny_http::Response::from_file(file));
                    }
                }
            }
        });
        format!("http://{addr}")
    }

    /// Two real builds: GL_DELTA_TEST=<dir> holding old/ (a raw publish of the installed build) and
    /// new/ (a glb1 publish of the next one), both as publish_build.py writes them. The old build is
    /// hardlinked into <dir>/work and updated file by file. Run with
    ///   CARGO_PROFILE_TEST_OPT_LEVEL=3 cargo test -- --ignored realdata --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn realdata() {
        let Ok(root) = std::env::var("GL_DELTA_TEST") else { return };
        let root = std::path::PathBuf::from(root);
        let manifest_in = |sub: &str| -> crate::net::Manifest {
            let p = std::fs::read_dir(root.join(sub).join("builds/linux"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p.file_name().unwrap() != "latest.json")
                .unwrap();
            serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
        };
        let (old, new) = (manifest_in("old"), manifest_in("new"));
        let work = root.join("work");
        let _ = std::fs::remove_dir_all(&work);
        for f in &old.files {
            let dst = work.join(&f.path);
            std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
            let src = root.join("old/objects").join(&f.sha256);
            if std::fs::hard_link(&src, &dst).is_err() {
                std::fs::copy(&src, &dst).unwrap();
            }
        }
        let url = serve_dir(root.join("new/blobs"));
        let client = reqwest::Client::new();
        let block = new.block.unwrap();
        let (mut total_net, t_all) = (0u64, std::time::Instant::now());
        for f in &new.files {
            let dest = work.join(&f.path);
            let c = Arc::new(Counter(AtomicU64::new(0), AtomicU64::new(0)));
            let t0 = std::time::Instant::now();
            fetch_blob(&client, &format!("{url}/{}", f.sha256), f.size, &f.sha256, block, &dest, tick(&c)).await.unwrap();
            let net = c.1.load(Ordering::Relaxed);
            total_net += net;
            eprintln!(
                "{:>9.1} MiB  net {:>8.1} MiB  {:>6.1} s  {}",
                f.size as f64 / 1048576.0,
                net as f64 / 1048576.0,
                t0.elapsed().as_secs_f64(),
                f.path
            );
        }
        eprintln!(
            "TOTAL {:.2} GiB, downloaded {:.1} MiB in {:.0} s",
            new.total as f64 / 2f64.powi(30),
            total_net as f64 / 1048576.0,
            t_all.elapsed().as_secs_f64()
        );
        for f in &new.files {
            assert_eq!(sha256_of(&work.join(&f.path)).unwrap(), f.sha256, "{}", f.path);
        }
    }

    #[test]
    fn runs_absorb_short_gaps() {
        let mut f = vec![true; 40];
        for i in [3, 4, 5, 9, 20, 39] {
            f[i] = false;
        }
        assert_eq!(missing_runs(&f, 2), vec![(3, 5), (9, 9), (20, 20), (39, 39)]);
        assert_eq!(missing_runs(&f, 3), vec![(3, 9), (20, 20), (39, 39)]);
        assert_eq!(missing_runs(&[true, true], 8), vec![]);
    }
}
