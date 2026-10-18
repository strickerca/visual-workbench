use crate::{AssetError, Pyramid, Result, TileKey};

const MAGIC: &[u8; 8] = b"VWGTILE1";
const HEADER: usize = 8 + 32 + 8 + 4 + 4 + 1 + 1 + 1 + 1 + 32 + 4;
const RECORD: usize = 36;
pub const MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_ENCODED_TILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_PYRAMID_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Original metadata, with oriented document dimensions. Cache pixels are always
/// 8-bit sRGB. Any source depth/profile conversion must be explicitly declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub original: [u8; 32],
    pub original_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
    pub bit_depth: u8,
    pub icc: Option<[u8; 32]>,
    pub display_conversion: bool,
}
impl Source {
    pub fn validate(&self) -> Result<Pyramid> {
        if self.original_bytes == 0 || self.original_bytes > 4 * 1024 * 1024 * 1024 || !(1..=8).contains(&self.orientation) || !matches!(self.bit_depth, 8 | 16) {
            return Err(AssetError::Invalid("source metadata"));
        }
        if !self.display_conversion && (self.bit_depth != 8 || self.icc.is_some()) { return Err(AssetError::Invalid("undeclared display conversion")); }
        Pyramid::new(self.width, self.height)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileRecord { pub key: TileKey, pub encoded_bytes: u32, pub hash: [u8; 32] }
impl TileRecord {
    pub fn relative_path(&self) -> String {
        format!("L{}/{}_{}.{}", self.key.level, self.key.x, self.key.y, if self.key.level == 0 { "webp" } else { "jpg" })
    }
    pub fn verify(&self, bytes: &[u8]) -> Result<()> {
        if bytes.len() != self.encoded_bytes as usize || blake3::hash(bytes).as_bytes() != &self.hash { return Err(AssetError::Binding); }
        Ok(())
    }
}

/// A completed immutable derived-cache inventory. Alpha is currently refused by
/// builders: JPEG lower levels cannot silently discard transparency. Neither
/// derived tiles nor this manifest are authoritative original project assets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest { source: Source, pyramid: Pyramid, tiles: Vec<TileRecord>, total: u64 }
impl Manifest {
    pub fn new(source: Source, tiles: Vec<TileRecord>) -> Result<Self> {
        let pyramid = source.validate()?;
        if tiles.len() != pyramid.tile_count() { return Err(AssetError::Invalid("tile inventory")); }
        let mut total = 0u64;
        for (expected, tile) in pyramid.keys().zip(&tiles) {
            if expected != tile.key || tile.encoded_bytes == 0 || tile.encoded_bytes as usize > MAX_ENCODED_TILE_BYTES { return Err(AssetError::Invalid("tile inventory")); }
            total = total.checked_add(u64::from(tile.encoded_bytes)).ok_or(AssetError::Limit("pyramid bytes"))?;
            if total > MAX_PYRAMID_BYTES { return Err(AssetError::Limit("pyramid bytes")); }
        }
        Ok(Self { source, pyramid, tiles, total })
    }
    pub fn source(&self) -> &Source { &self.source }
    pub fn pyramid(&self) -> &Pyramid { &self.pyramid }
    pub fn tiles(&self) -> &[TileRecord] { &self.tiles }
    pub fn encoded_bytes(&self) -> u64 { self.total }
    pub fn tile(&self, key: TileKey) -> Result<&TileRecord> {
        self.pyramid.region(key)?;
        let prefix: usize = self.pyramid.levels().iter().take(key.level as usize).map(|l| (u64::from(l.columns) * u64::from(l.rows)) as usize).sum();
        let level = self.pyramid.level(key.level)?;
        self.tiles.get(prefix + (u64::from(key.y) * u64::from(level.columns) + u64::from(key.x)) as usize).ok_or(AssetError::Invalid("tile inventory"))
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + self.tiles.len() * RECORD + 32);
        out.extend_from_slice(MAGIC); out.extend_from_slice(&self.source.original);
        out.extend_from_slice(&self.source.original_bytes.to_le_bytes());
        out.extend_from_slice(&self.source.width.to_le_bytes()); out.extend_from_slice(&self.source.height.to_le_bytes());
        out.push(self.source.orientation); out.push(self.source.bit_depth);
        out.push(u8::from(self.source.display_conversion)); out.push(u8::from(self.source.icc.is_some()));
        out.extend_from_slice(&self.source.icc.unwrap_or([0; 32]));
        out.extend_from_slice(&(self.tiles.len() as u32).to_le_bytes());
        for tile in &self.tiles { out.extend_from_slice(&tile.encoded_bytes.to_le_bytes()); out.extend_from_slice(&tile.hash); }
        let hash = blake3::hash(&out); out.extend_from_slice(hash.as_bytes()); out
    }
    pub fn identity(&self) -> [u8; 32] { *blake3::hash(&self.encode()).as_bytes() }
    /// Before any collection allocation, admit exact fixed-width length, source
    /// dimensions, tile count and checksum. No attacker-selected strings/paths.
    pub fn decode(bytes: &[u8], expected_source: &Source) -> Result<Self> {
        if bytes.len() < HEADER + 32 || bytes.len() > MAX_MANIFEST_BYTES { return Err(AssetError::Limit("manifest bytes")); }
        if &bytes[..8] != MAGIC { return Err(AssetError::Invalid("manifest version")); }
        let payload = &bytes[..bytes.len() - 32];
        if blake3::hash(payload).as_bytes().as_slice() != &bytes[bytes.len() - 32..] { return Err(AssetError::Binding); }
        let mut cursor = Cursor { bytes: payload, at: 8 };
        let original = cursor.array()?; let original_bytes = u64::from_le_bytes(cursor.array()?);
        let width = u32::from_le_bytes(cursor.array()?); let height = u32::from_le_bytes(cursor.array()?);
        let orientation = cursor.array::<1>()?[0]; let bit_depth = cursor.array::<1>()?[0];
        let display_conversion = boolean(cursor.array::<1>()?[0])?; let has_icc = boolean(cursor.array::<1>()?[0])?;
        let profile = cursor.array::<32>()?;
        if !has_icc && profile != [0;32] { return Err(AssetError::Invalid("absent profile")); }
        let source = Source { original, original_bytes, width, height, orientation, bit_depth, icc: has_icc.then_some(profile), display_conversion };
        if &source != expected_source { return Err(AssetError::Binding); }
        let pyramid = source.validate()?;
        let count = u32::from_le_bytes(cursor.array()?) as usize;
        if count != pyramid.tile_count() || HEADER + count * RECORD != payload.len() { return Err(AssetError::Invalid("manifest count/length")); }
        let mut tiles = Vec::with_capacity(count);
        for key in pyramid.keys() { tiles.push(TileRecord { key, encoded_bytes: u32::from_le_bytes(cursor.array()?), hash: cursor.array()? }); }
        Self::new(source, tiles)
    }
}
fn boolean(v: u8) -> Result<bool> { match v { 0 => Ok(false), 1 => Ok(true), _ => Err(AssetError::Invalid("manifest boolean")) } }
struct Cursor<'a> { bytes: &'a [u8], at: usize }
impl Cursor<'_> {
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let slice = self.bytes.get(self.at..self.at + N).ok_or(AssetError::Invalid("manifest truncated"))?;
        let mut out = [0; N]; out.copy_from_slice(slice); self.at += N; Ok(out)
    }
}
