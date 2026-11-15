//! The canonical state hash, recomputed from where a project last changed.
//!
//! `Project::state_hash` serializes and hashes every byte of the project. The
//! canonical bytes are one JSON object, and the hash is BLAKE3 over them, so
//! two things can be kept between one hash and the next: the bytes of every
//! entity that did not change, and the hash state in front of the first byte
//! that did. An edit then serializes only the entities it touched, in every
//! member, and hashes from there to the end. New objects carry time-ordered
//! IDs and land at the end of the `objects` member: saving one more stroke
//! hashes that stroke and the short remainder of the project.
//!
//! What cannot be saved is the pass over the bytes behind an edit. BLAKE3
//! binds every kilobyte to its position, so a change to an early object, or
//! to a member written before the objects (a layer, an instruction, a
//! document, a mask history), moves everything behind it and all of that is
//! hashed again, from memory, at a few gigabytes a second, after the kept
//! bytes of the members in front of the objects are put together again,
//! which is a copy. Nothing behind the edit is serialized or validated again.
//! `work` says how much of each the last hash did.
use crate::{
    AssetId, DeviceId, Id, ModelError, Object, Project, Replaced, StateHash, state::normalize,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use vw_proto::v1;

pub(crate) const DOMAIN: &[u8] = b"VisualWorkbench.Project.v1\0";
/// Bytes of object entries a page is filled to. Hashing resumes at the page
/// that holds the first changed object, and BLAKE3 is fastest when it is fed
/// pieces of this size rather than one object at a time.
const PAGE: usize = 256 * 1024;
/// A page that grows past this by insertions in its middle is cut in two.
const PAGE_SPLIT: usize = 2 * PAGE;
/// The text of an ID, and where an entry keeps it: `,"<id>":{…}`.
const ID_LEN: usize = 36;
const ID_AT: usize = 2;

/// What one hash did: the bytes it serialized afresh (entries of the
/// entities that changed, or everything the first time), about how many
/// JSON values those bytes hold (a long text is one, a list of numbers is
/// one per number, and values cost more than bytes), and the bytes it fed
/// to BLAKE3.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HashWork {
    pub serialized: usize,
    pub values: usize,
    pub hashed: usize,
}
impl HashWork {
    /// Freshly serialized bytes; their separators count the values.
    fn serialized(&mut self, bytes: &[u8]) {
        self.serialized += bytes.len();
        self.values += bytes.iter().filter(|byte| **byte == b',').count() + 1;
    }
}

/// Remembers the canonical bytes of one project and how far they were hashed.
///
/// Give it the same project every time and report each change first:
/// `replaced` with what an edit replaced, or `objects_changed`,
/// `objects_changed_from` and `member_changed` for the parts, or `reset` when
/// the project is another one. The result is bit-identical to
/// `Project::state_hash`. It does not validate.
///
/// It holds a copy of the canonical bytes, about as much memory again as the
/// project itself takes.
#[derive(Debug, Clone, Default)]
pub struct StateHasher {
    known: Option<Known>,
    /// Objects created, changed or removed since the last hash.
    objects: BTreeSet<Id>,
    /// Every object from this ID on may have changed since the last hash.
    objects_from: Option<Id>,
    /// Members other than `objects` changed since the last hash, by
    /// position: the keys of the entities that changed, or all of them.
    members: BTreeMap<usize, Option<BTreeSet<String>>>,
    work: HashWork,
}

/// The canonical bytes as of the last hash.
#[derive(Debug, Clone)]
struct Known {
    outline: Outline,
    /// The entries of the `objects` member in ID order. No page is empty.
    pages: Vec<Page>,
    /// The leading pages whose `before` is the state in front of them.
    fresh: usize,
    /// The hash state behind the last object entry, when it is current.
    end: Option<blake3::Hasher>,
    /// Bytes in all pages, the comma in front of the first entry included.
    bytes: usize,
    /// How many bytes longer all entries are when zeros are not normalized.
    zeros: usize,
    /// The length of the project as plain JSON, as of the last hash.
    json_len: usize,
}

/// Consecutive object entries, each written `,"<id>":{…}`. The canonical
/// bytes have no comma in front of the first entry of the member, which is
/// left out when the first page is hashed.
#[derive(Debug, Clone)]
struct Page {
    /// The hash state in front of this page's bytes.
    before: blake3::Hasher,
    bytes: Vec<u8>,
    /// Where each entry ends in `bytes`.
    ends: Vec<u32>,
    /// How many bytes longer each entry is when its zeros are not normalized.
    zeros: Vec<u32>,
}

fn page_error() -> ModelError {
    ModelError::Invalid("canonical object pages")
}

impl Page {
    fn start(&self, index: usize) -> usize {
        index
            .checked_sub(1)
            .and_then(|previous| self.ends.get(previous))
            .map_or(0, |end| *end as usize)
    }
    /// The ID text of the entry at `index`.
    fn id(&self, index: usize) -> &[u8] {
        let start = self.start(index) + ID_AT;
        self.bytes.get(start..start + ID_LEN).unwrap_or_default()
    }
    /// Where `id` is, or where it would be inserted.
    fn find(&self, id: &Id) -> Result<usize, usize> {
        let (mut low, mut high) = (0, self.ends.len());
        while low < high {
            let middle = low + (high - low) / 2;
            match self.id(middle).cmp(id.as_str().as_bytes()) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => return Ok(middle),
            }
        }
        Err(low)
    }
    /// Replace the entries `range` with `entry` (empty to remove them).
    /// Returns the zero bytes of the entries that went.
    fn splice(
        &mut self,
        range: std::ops::Range<usize>,
        entry: &[u8],
        zeros: u32,
    ) -> Result<usize, ModelError> {
        if range.end > self.ends.len() || range.start > range.end {
            return Err(page_error());
        }
        let (from, to) = (self.start(range.start), self.start(range.end));
        self.bytes.splice(from..to, entry.iter().copied());
        let end = u32::try_from(from + entry.len())
            .map_err(|_| ModelError::Invalid("canonical object size"))?;
        let removed = to - from;
        let inserted = if entry.is_empty() { None } else { Some(end) };
        self.ends.splice(range.clone(), inserted);
        let gone = self
            .zeros
            .splice(range.clone(), inserted.map(|_| zeros))
            .map(|zeros| zeros as usize)
            .sum();
        let next = range.start + usize::from(!entry.is_empty());
        for later in self.ends.iter_mut().skip(next) {
            *later = u32::try_from(*later as usize - removed + entry.len())
                .map_err(|_| ModelError::Invalid("canonical object size"))?;
        }
        Ok(gone)
    }
    /// Cut the page in two at the entry nearest its middle.
    fn split(&mut self) -> Option<Self> {
        let half = self.bytes.len() / 2;
        let at = self.ends.partition_point(|end| (*end as usize) <= half);
        if at == 0 || at >= self.ends.len() {
            return None;
        }
        let cut = *self.ends.get(at - 1)?;
        let bytes = self.bytes.split_off(cut as usize);
        let ends = self
            .ends
            .split_off(at)
            .into_iter()
            .map(|end| end - cut)
            .collect();
        Some(Self {
            before: blake3::Hasher::new(),
            bytes,
            ends,
            zeros: self.zeros.split_off(at),
        })
    }
    /// The bytes of the page as the canonical bytes hold them.
    fn hashed(&self, first: bool) -> &[u8] {
        if first {
            self.bytes.get(1..).unwrap_or_default()
        } else {
            &self.bytes
        }
    }
}

impl StateHasher {
    /// An edit replaced these entities; see `Replaced`.
    pub fn replaced(&mut self, replaced: &Replaced) {
        self.objects_changed(replaced.objects.keys());
        macro_rules! member {
            ($field:ident, $name:literal) => {
                if !replaced.$field.is_empty() {
                    self.entries_changed($name, replaced.$field.keys().map(|key| key.as_str()));
                }
            };
        }
        member!(documents, "documents");
        member!(assets, "assets");
        member!(layers, "layers");
        member!(groups, "groups");
        member!(instructions, "instructions");
        member!(semantic_snapshots, "semantic_snapshots");
        member!(results, "results");
        member!(mask_versions, "mask_versions");
    }

    /// These objects were created, changed or removed.
    pub fn objects_changed<'a>(&mut self, ids: impl IntoIterator<Item = &'a Id>) {
        if self.known.is_some() {
            self.objects.extend(ids.into_iter().cloned());
        }
    }

    /// Objects with this ID or a later one were created, changed or removed;
    /// nothing else in the project changed.
    pub fn objects_changed_from(&mut self, first: &Id) {
        if self.known.is_some() && self.objects_from.as_ref().is_none_or(|from| first < from) {
            self.objects_from = Some(first.clone());
        }
    }

    /// A member of the project other than `objects` changed: `layers`,
    /// `instructions`, and so on, by its name in the project.
    pub fn member_changed(&mut self, name: &str) {
        let Some(known) = &self.known else {
            return;
        };
        match known.outline.position(name) {
            Some(position) => {
                self.members.insert(position, None);
            }
            // Not a member this hasher can refresh on its own.
            None => self.reset(),
        }
    }

    /// The entities with these keys of a member other than `objects` were
    /// created, changed or removed.
    fn entries_changed<'a>(&mut self, name: &str, keys: impl Iterator<Item = &'a str>) {
        let Some(known) = &self.known else {
            return;
        };
        let Some(position) = known.outline.position(name) else {
            self.reset();
            return;
        };
        if !known.outline.keyed(position) {
            self.members.insert(position, None);
            return;
        }
        if let Some(changed) = self
            .members
            .entry(position)
            .or_insert_with(|| Some(BTreeSet::new()))
        {
            changed.extend(keys.map(str::to_owned));
        }
    }

    /// The project was replaced, or changed in a way not reported.
    pub fn reset(&mut self) {
        self.known = None;
        self.objects.clear();
        self.objects_from = None;
        self.members.clear();
    }

    /// The length of the project hashed last when serde_json writes it as it
    /// is, without normalizing zeros: what it takes inside a larger JSON
    /// document. Nothing before the first hash or after a failed one.
    pub fn json_len(&self) -> Option<usize> {
        self.known.as_ref().map(|known| known.json_len)
    }

    /// What the last hash did.
    pub const fn work(&self) -> HashWork {
        self.work
    }

    /// The state hash of `project` and the length of its canonical bytes.
    pub fn hash(&mut self, project: &Project) -> Result<(StateHash, usize), ModelError> {
        self.work = HashWork::default();
        let result = self.follow(project);
        if result.is_err() {
            // Whatever was half brought up to date is not trusted again.
            self.reset();
        }
        result
    }

    fn follow(&mut self, project: &Project) -> Result<(StateHash, usize), ModelError> {
        let objects = std::mem::take(&mut self.objects);
        let from = self.objects_from.take();
        let members = std::mem::take(&mut self.members);
        let work = &mut self.work;
        let known = match &mut self.known {
            Some(known) => {
                for (position, keys) in members {
                    known.refresh_member(project, position, keys.as_ref(), work)?;
                }
                // Everything from `from` on is read again; single objects
                // before it are brought up to date one by one.
                for id in &objects {
                    if from.as_ref().is_none_or(|from| id < from) {
                        known.sync(project, id, work)?;
                    }
                }
                if let Some(from) = &from {
                    known.truncate(from);
                    known.extend(project, Some(from), work)?;
                }
                known
            }
            None => {
                let outline = Outline::of(project)?;
                work.serialized(&outline.head);
                work.serialized(&outline.tail);
                let mut known = Known {
                    outline,
                    pages: Vec::new(),
                    fresh: 0,
                    end: None,
                    bytes: 0,
                    zeros: 0,
                    json_len: 0,
                };
                known.extend(project, None, work)?;
                self.known.insert(known)
            }
        };
        Ok(known.finish(work))
    }
}

impl Known {
    /// The page that holds `id` or would take it.
    fn page_of(&self, id: &Id) -> usize {
        self.pages
            .partition_point(|page| page.id(0) <= id.as_str().as_bytes())
            .saturating_sub(1)
    }

    /// Page `index` changed: the states in front of the pages behind it, and
    /// the one behind the last entry, are no longer current.
    fn changed(&mut self, index: usize) {
        self.fresh = self.fresh.min(index + 1);
        self.end = None;
    }

    /// Bring the entry of one object up to date with the project.
    fn sync(&mut self, project: &Project, id: &Id, work: &mut HashWork) -> Result<(), ModelError> {
        let mut entry = Vec::new();
        let mut zeros = 0;
        if let Some(object) = project.objects.get(id) {
            zeros = write_entry(&mut entry, id, object)?;
            work.serialized(&entry);
        }
        if self.pages.is_empty() {
            return self.append(&entry, zeros, work);
        }
        let index = self.page_of(id);
        let count = self.pages.len();
        let page = self.pages.get_mut(index).ok_or_else(page_error)?;
        match page.find(id) {
            Ok(position) => {
                let old = page.start(position + 1) - page.start(position);
                let gone = page.splice(position..position + 1, &entry, zeros)?;
                let empty = page.ends.is_empty();
                self.bytes = self.bytes - old + entry.len();
                self.zeros -= gone;
                if !entry.is_empty() {
                    self.zeros += zeros as usize;
                }
                self.changed(index);
                if empty {
                    self.pages.remove(index);
                    self.fresh = self.fresh.min(index);
                }
            }
            Err(_) if entry.is_empty() => {}
            Err(position) => {
                if index + 1 == count && position == page.ends.len() {
                    return self.append(&entry, zeros, work);
                }
                page.splice(position..position, &entry, zeros)?;
                let second = if page.bytes.len() > PAGE_SPLIT {
                    page.split()
                } else {
                    None
                };
                self.bytes += entry.len();
                self.zeros += zeros as usize;
                self.changed(index);
                if let Some(second) = second {
                    self.pages.insert(index + 1, second);
                }
            }
        }
        Ok(())
    }

    /// Add an entry behind every other. The state behind the last entry, when
    /// it is current, takes the new bytes and stays current.
    fn append(&mut self, entry: &[u8], zeros: u32, work: &mut HashWork) -> Result<(), ModelError> {
        if entry.is_empty() {
            return Ok(());
        }
        let first = self.pages.is_empty();
        let room = self
            .pages
            .last()
            .is_some_and(|page| page.bytes.len() < PAGE);
        let current = self.fresh == self.pages.len();
        if !room {
            // A page that starts where every earlier byte is already hashed.
            let before = match (&self.end, current) {
                (Some(end), true) => {
                    self.fresh += 1;
                    end.clone()
                }
                _ => blake3::Hasher::new(),
            };
            self.pages.push(Page {
                before,
                bytes: Vec::new(),
                ends: Vec::new(),
                zeros: Vec::new(),
            });
        }
        let index = self.pages.len() - 1;
        let page = self.pages.last_mut().ok_or_else(page_error)?;
        page.bytes.extend_from_slice(entry);
        page.ends.push(
            u32::try_from(page.bytes.len())
                .map_err(|_| ModelError::Invalid("canonical object size"))?,
        );
        page.zeros.push(zeros);
        self.bytes += entry.len();
        self.zeros += zeros as usize;
        match &mut self.end {
            Some(end) if self.fresh == self.pages.len() => {
                let bytes = if first {
                    entry.get(1..).unwrap_or_default()
                } else {
                    entry
                };
                end.update(bytes);
                work.hashed += bytes.len();
            }
            _ => self.changed(index),
        }
        Ok(())
    }

    /// Forget every entry from `from` on.
    fn truncate(&mut self, from: &Id) {
        let index = self.page_of(from);
        let Some(page) = self.pages.get(index) else {
            return;
        };
        let position = match page.find(from) {
            Ok(position) | Err(position) => position,
        };
        let cut = page.start(position);
        let later = self.pages.get(index + 1..).unwrap_or_default();
        let removed =
            page.bytes.len() - cut + later.iter().map(|page| page.bytes.len()).sum::<usize>();
        if removed == 0 {
            return;
        }
        let zeros = page
            .zeros
            .get(position..)
            .unwrap_or_default()
            .iter()
            .chain(later.iter().flat_map(|page| &page.zeros))
            .map(|zeros| *zeros as usize)
            .sum::<usize>();
        self.bytes -= removed;
        self.zeros -= zeros;
        self.pages.truncate(index + 1);
        if let Some(page) = self.pages.get_mut(index) {
            page.bytes.truncate(cut);
            page.ends.truncate(position);
            page.zeros.truncate(position);
        }
        self.changed(index);
        if position == 0 {
            self.pages.pop();
            self.fresh = self.fresh.min(index);
        }
    }

    /// Append the entries of the project's objects from `from` on.
    fn extend(
        &mut self,
        project: &Project,
        from: Option<&Id>,
        work: &mut HashWork,
    ) -> Result<(), ModelError> {
        let mut entry = Vec::new();
        let objects = match from {
            Some(from) => project.objects.range::<Id, _>(from..),
            None => project.objects.range::<Id, _>(..),
        };
        for (id, object) in objects {
            entry.clear();
            let zeros = write_entry(&mut entry, id, object)?;
            work.serialized(&entry);
            self.append(&entry, zeros, work)?;
        }
        Ok(())
    }

    /// Serialize the changed entities of one member again; hashing restarts
    /// in front of it when it is written before the objects.
    fn refresh_member(
        &mut self,
        project: &Project,
        position: usize,
        keys: Option<&BTreeSet<String>>,
        work: &mut HashWork,
    ) -> Result<(), ModelError> {
        if self.outline.refresh(project, position, keys, work)? {
            self.fresh = 0;
            self.end = None;
        }
        Ok(())
    }

    /// Hash from the first byte that is not hashed yet to the end.
    fn finish(&mut self, work: &mut HashWork) -> (StateHash, usize) {
        let end = match self.end.take() {
            Some(end) => end,
            None => {
                let resume = self
                    .fresh
                    .checked_sub(1)
                    .and_then(|fresh| Some((fresh, self.pages.get(fresh)?.before.clone())));
                let mut hasher = match resume {
                    Some((fresh, before)) => {
                        self.fresh = fresh;
                        before
                    }
                    None => {
                        self.fresh = 0;
                        let mut hasher = blake3::Hasher::new();
                        hasher.update(DOMAIN);
                        hasher.update(&self.outline.head);
                        work.hashed += DOMAIN.len() + self.outline.head.len();
                        hasher
                    }
                };
                for (index, page) in self.pages.iter_mut().enumerate().skip(self.fresh) {
                    page.before = hasher.clone();
                    let bytes = page.hashed(index == 0);
                    hasher.update(bytes);
                    work.hashed += bytes.len();
                }
                self.fresh = self.pages.len();
                hasher
            }
        };
        let mut hasher = end.clone();
        self.end = Some(end);
        hasher.update(&self.outline.tail);
        work.hashed += self.outline.tail.len();
        // The comma in front of the first entry is not part of the bytes.
        let length =
            self.outline.head.len() + self.bytes.saturating_sub(1) + self.outline.tail.len();
        self.json_len = length + self.zeros + self.outline.zeros();
        (StateHash::from_digest(hasher.finalize()), length)
    }
}

/// `Project` with the `objects` member left empty. Listing every field makes a
/// new project field a compile error here rather than a silently wrong hash.
#[derive(Serialize)]
struct Shell<'a> {
    id: &'a Id,
    title: &'a str,
    created_by: &'a DeviceId,
    schema_version: u32,
    documents: &'a BTreeMap<Id, crate::Document>,
    assets: &'a BTreeMap<AssetId, v1::AddAsset>,
    layers: &'a BTreeMap<Id, crate::Layer>,
    objects: BTreeMap<Id, Object>,
    groups: &'a BTreeMap<Id, crate::Group>,
    instructions: &'a BTreeMap<Id, crate::Instruction>,
    semantic_snapshots: &'a BTreeMap<Id, crate::SemanticSnapshot>,
    results: &'a BTreeMap<Id, crate::ResultCandidate>,
    mask_versions: &'a BTreeMap<Id, Vec<v1::MaskOp>>,
}

/// The canonical bytes around the entries of the `objects` member, kept
/// member by member and, for the members that map IDs to entities, entity
/// by entity, so that a changed entity is serialized alone.
#[derive(Debug, Clone)]
struct Outline {
    /// Every member in the order serde_json writes them.
    members: Vec<Member>,
    /// The position of `objects` among them.
    objects: usize,
    /// Everything in front of the first object entry.
    head: Vec<u8>,
    /// Everything behind the last one.
    tail: Vec<u8>,
}

/// One member of the project.
#[derive(Debug, Clone)]
struct Member {
    name: String,
    /// `"name":value`; for `objects` only `"objects":{`.
    bytes: Vec<u8>,
    /// How many bytes longer the value is when its zeros are not normalized.
    zeros: usize,
    /// For a member that maps IDs to entities: `"key":value` of each entity
    /// and its zeros, from which `bytes` is put together again.
    entries: Option<BTreeMap<String, (Vec<u8>, usize)>>,
}

impl Member {
    /// Put `bytes` together again from the entries.
    fn join_entries(&mut self) -> Result<(), ModelError> {
        let Some(entries) = &self.entries else {
            return Ok(());
        };
        let size = entries
            .values()
            .map(|(bytes, _)| bytes.len() + 1)
            .sum::<usize>();
        let mut bytes = Vec::with_capacity(self.name.len() + 4 + size);
        write_name(&mut bytes, &self.name)?;
        bytes.push(b'{');
        for (index, (entry, _)) in entries.values().enumerate() {
            if index > 0 {
                bytes.push(b',');
            }
            bytes.extend_from_slice(entry);
        }
        bytes.push(b'}');
        self.bytes = bytes;
        self.zeros = entries.values().map(|(_, zeros)| zeros).sum();
        Ok(())
    }
}

/// The members that map IDs to entities, other than `objects`.
const KEYED: [&str; 8] = [
    "documents",
    "assets",
    "layers",
    "groups",
    "instructions",
    "semantic_snapshots",
    "results",
    "mask_versions",
];

/// One entity of a keyed member as a JSON value, if the project has it.
fn entity(
    project: &Project,
    member: &str,
    key: &str,
) -> Result<Option<serde_json::Value>, ModelError> {
    fn get<K: Ord + TryFrom<String>, V: Serialize>(
        map: &BTreeMap<K, V>,
        key: &str,
    ) -> Result<Option<serde_json::Value>, ModelError> {
        let Ok(key) = K::try_from(key.to_owned()) else {
            return Ok(None);
        };
        Ok(map.get(&key).map(serde_json::to_value).transpose()?)
    }
    match member {
        "documents" => get(&project.documents, key),
        "assets" => get(&project.assets, key),
        "layers" => get(&project.layers, key),
        "groups" => get(&project.groups, key),
        "instructions" => get(&project.instructions, key),
        "semantic_snapshots" => get(&project.semantic_snapshots, key),
        "results" => get(&project.results, key),
        "mask_versions" => get(&project.mask_versions, key),
        _ => Err(ModelError::Invalid("canonical project outline")),
    }
}

/// `"key":value`, normalized, and how many bytes the zeros took out.
fn write_keyed(
    out: &mut Vec<u8>,
    key: &str,
    value: &mut serde_json::Value,
) -> Result<usize, ModelError> {
    let zeros = normalize(value);
    write_name(out, key)?;
    serde_json::to_writer(&mut *out, value)?;
    Ok(zeros)
}

impl Outline {
    fn zeros(&self) -> usize {
        self.members.iter().map(|member| member.zeros).sum()
    }

    fn of(project: &Project) -> Result<Self, ModelError> {
        let Project {
            id,
            title,
            created_by,
            schema_version,
            documents,
            assets,
            layers,
            objects: _,
            groups,
            instructions,
            semantic_snapshots,
            results,
            mask_versions,
        } = project;
        let mut value = serde_json::to_value(Shell {
            id,
            title,
            created_by,
            schema_version: *schema_version,
            documents,
            assets,
            layers,
            objects: BTreeMap::new(),
            groups,
            instructions,
            semantic_snapshots,
            results,
            mask_versions,
        })?;
        let serde_json::Value::Object(values) = &mut value else {
            return Err(ModelError::Invalid("canonical project outline"));
        };
        let mut members = Vec::with_capacity(values.len());
        let mut objects = None;
        for (index, (name, member)) in values.iter_mut().enumerate() {
            let mut bytes = Vec::new();
            let mut entries = None;
            let zeros = if name == "objects" {
                objects = Some(index);
                write_name(&mut bytes, name)?;
                bytes.push(b'{');
                0
            } else if KEYED.contains(&name.as_str())
                && let serde_json::Value::Object(map) = member
            {
                let mut keyed = BTreeMap::new();
                for (key, value) in map.iter_mut() {
                    let mut entry = Vec::new();
                    let zeros = write_keyed(&mut entry, key, value)?;
                    keyed.insert(key.clone(), (entry, zeros));
                }
                entries = Some(keyed);
                0
            } else {
                let zeros = normalize(member);
                write_member(&mut bytes, name, member)?;
                zeros
            };
            let mut member = Member {
                name: name.clone(),
                bytes,
                zeros,
                entries,
            };
            member.join_entries()?;
            members.push(member);
        }
        let mut outline = Self {
            members,
            objects: objects.ok_or(ModelError::Invalid("canonical project outline"))?,
            head: Vec::new(),
            tail: Vec::new(),
        };
        outline.join(true, true);
        Ok(outline)
    }

    /// The position of a member this outline can serialize on its own.
    fn position(&self, name: &str) -> Option<usize> {
        self.members
            .iter()
            .position(|member| member.name == name)
            .filter(|position| *position != self.objects)
    }

    /// Whether the member at `position` is kept entity by entity.
    fn keyed(&self, position: usize) -> bool {
        self.members
            .get(position)
            .is_some_and(|member| member.entries.is_some())
    }

    /// Serialize again the entities `keys` names of the member at
    /// `position`, or all of it. True when it is written in front of the
    /// objects, so that every byte behind it has moved.
    fn refresh(
        &mut self,
        project: &Project,
        position: usize,
        keys: Option<&BTreeSet<String>>,
        work: &mut HashWork,
    ) -> Result<bool, ModelError> {
        let member = self
            .members
            .get_mut(position)
            .ok_or(ModelError::Invalid("canonical project outline"))?;
        match (&mut member.entries, keys) {
            (Some(entries), Some(keys)) => {
                for key in keys {
                    match entity(project, &member.name, key)? {
                        Some(mut value) => {
                            let mut entry = Vec::new();
                            let zeros = write_keyed(&mut entry, key, &mut value)?;
                            work.serialized(&entry);
                            entries.insert(key.clone(), (entry, zeros));
                        }
                        None => {
                            entries.remove(key);
                        }
                    }
                }
                member.join_entries()?;
            }
            (entries, _) => {
                let mut value = match member.name.as_str() {
                    "documents" => serde_json::to_value(&project.documents)?,
                    "assets" => serde_json::to_value(&project.assets)?,
                    "layers" => serde_json::to_value(&project.layers)?,
                    "groups" => serde_json::to_value(&project.groups)?,
                    "instructions" => serde_json::to_value(&project.instructions)?,
                    "semantic_snapshots" => serde_json::to_value(&project.semantic_snapshots)?,
                    "results" => serde_json::to_value(&project.results)?,
                    "mask_versions" => serde_json::to_value(&project.mask_versions)?,
                    "id" => serde_json::to_value(&project.id)?,
                    "title" => serde_json::to_value(&project.title)?,
                    "created_by" => serde_json::to_value(&project.created_by)?,
                    "schema_version" => serde_json::to_value(project.schema_version)?,
                    _ => return Err(ModelError::Invalid("canonical project outline")),
                };
                match (entries, &mut value) {
                    (Some(entries), serde_json::Value::Object(map)) => {
                        entries.clear();
                        for (key, value) in map.iter_mut() {
                            let mut entry = Vec::new();
                            let zeros = write_keyed(&mut entry, key, value)?;
                            entries.insert(key.clone(), (entry, zeros));
                        }
                        member.join_entries()?;
                    }
                    (None, value) => {
                        member.zeros = normalize(value);
                        member.bytes.clear();
                        write_member(&mut member.bytes, &member.name, value)?;
                    }
                    (Some(_), _) => {
                        return Err(ModelError::Invalid("canonical project outline"));
                    }
                }
                work.serialized(&member.bytes);
            }
        }
        let before = position < self.objects;
        self.join(before, !before);
        Ok(before)
    }

    /// Write the members exactly as serde_json writes the whole value, and
    /// split inside the braces of `objects`.
    fn join(&mut self, head: bool, tail: bool) {
        let (front, back) = self.members.split_at(self.objects.min(self.members.len()));
        if head {
            self.head.clear();
            self.head.push(b'{');
            for (index, member) in front.iter().chain(back.first()).enumerate() {
                if index > 0 {
                    self.head.push(b',');
                }
                self.head.extend_from_slice(&member.bytes);
            }
        }
        if tail {
            self.tail.clear();
            self.tail.push(b'}');
            for member in back.iter().skip(1) {
                self.tail.push(b',');
                self.tail.extend_from_slice(&member.bytes);
            }
            self.tail.push(b'}');
        }
    }
}

fn write_name(out: &mut Vec<u8>, name: &str) -> Result<(), ModelError> {
    serde_json::to_writer(&mut *out, name)?;
    out.push(b':');
    Ok(())
}

fn write_member(
    out: &mut Vec<u8>,
    name: &str,
    value: &serde_json::Value,
) -> Result<(), ModelError> {
    write_name(out, name)?;
    serde_json::to_writer(&mut *out, value)?;
    Ok(())
}

/// One `,"id":{object}` member of `objects`. Returns how many bytes longer
/// the object is when its zeros are not normalized.
fn write_entry(out: &mut Vec<u8>, id: &Id, object: &Object) -> Result<u32, ModelError> {
    out.push(b',');
    serde_json::to_writer(&mut *out, id)?;
    out.push(b':');
    let mut value = serde_json::to_value(object)?;
    let zeros = normalize(&mut value);
    serde_json::to_writer(&mut *out, &value)?;
    u32::try_from(zeros).map_err(|_| ModelError::Invalid("canonical object size"))
}
