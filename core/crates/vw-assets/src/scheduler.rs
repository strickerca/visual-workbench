use crate::{AssetError, Manifest, Result, TileKey, Viewport, ViewportTiles};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub manifest: [u8; 32],
    pub id: u64,
    pub viewport: u64,
    pub tile: TileKey,
}

/// Latest viewport + one prefetch ring, maximum eight actual requests in flight.
/// Switching views never frees an in-flight slot: caller must settle/cancel its
/// producer and call finish. Thus a slow peer cannot spawn unbounded old reads.
pub struct TileScheduler {
    manifest: Manifest,
    identity: [u8; 32],
    generation: u64,
    next: u64,
    capacity: usize,
    queue: VecDeque<TileKey>,
    wanted: BTreeSet<TileKey>,
    active: BTreeMap<u64, Request>,
}
impl TileScheduler {
    pub fn new(manifest: Manifest, capacity: usize) -> Result<Self> {
        if !(1..=8).contains(&capacity) {
            return Err(AssetError::Limit("tile concurrency"));
        }
        let identity = manifest.identity();
        Ok(Self {
            manifest,
            identity,
            generation: 0,
            next: 0,
            capacity,
            queue: VecDeque::new(),
            wanted: BTreeSet::new(),
            active: BTreeMap::new(),
        })
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn viewport(&mut self, view: Viewport) -> Result<ViewportTiles> {
        let tiles = self.manifest.pyramid().viewport(view)?;
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(AssetError::Limit("viewport generation"))?;
        self.queue = tiles
            .visible
            .iter()
            .chain(&tiles.prefetch)
            .copied()
            .collect();
        self.wanted = self.queue.iter().copied().collect();
        self.generation = generation;
        Ok(tiles)
    }
    pub fn next(&mut self, cached: impl Fn(TileKey) -> bool) -> Result<Option<Request>> {
        if self.active.len() >= self.capacity {
            return Ok(None);
        }
        while let Some(tile) = self.queue.pop_front() {
            if cached(tile) || self.active.values().any(|r| r.tile == tile) {
                continue;
            }
            self.next = self
                .next
                .checked_add(1)
                .ok_or(AssetError::Limit("request sequence"))?;
            let request = Request {
                manifest: self.identity,
                id: self.next,
                viewport: self.generation,
                tile,
            };
            self.active.insert(request.id, request);
            return Ok(Some(request));
        }
        Ok(None)
    }
    /// Exact completion consumes only its own active slot. true permits cache
    /// insertion for the current view after content verification; false discards
    /// old bytes. Forged/duplicate completion cannot release another producer.
    pub fn finish(&mut self, request: Request) -> Result<bool> {
        if self.active.get(&request.id) != Some(&request) {
            return Err(AssetError::Stale);
        }
        self.active.remove(&request.id);
        Ok(request.viewport == self.generation && self.wanted.contains(&request.tile))
    }
    pub fn retry(&mut self, request: Request) -> Result<()> {
        if self.finish(request)? {
            self.queue.push_front(request.tile);
        }
        Ok(())
    }
    pub fn cancel_view(&mut self) -> Result<()> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(AssetError::Limit("viewport generation"))?;
        self.queue.clear();
        self.wanted.clear();
        Ok(())
    }
    pub fn active(&self) -> impl Iterator<Item = &Request> {
        self.active.values()
    }
}
