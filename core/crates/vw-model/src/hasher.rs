//! The canonical state hash, recomputed from where a project last changed.
//!
//! `Project::state_hash` serializes and hashes every byte of the project. The
//! canonical bytes are one JSON object, and the hash is BLAKE3 over them, so
//! two things can be kept between one hash and the next: the bytes of every
//! entity that did not change, and the hash state in front of the first byte
//! that did. An edit then serializes only the entities it touched and hashes
//! from there to the end. New objects carry time-ordered IDs and land at the
//! end of the `objects` member: saving one more stroke hashes that stroke and
//! the short remainder of the project.
//!
//! What cannot be saved is the pass over the bytes behind an edit. BLAKE3
//! binds every kilobyte to its position, so a change to an early object, or
//! to a member written before the objects (a layer, an instruction, a
//! document, a mask history), moves everything behind it and all of that is
//! hashed again, from memory, at a few gigabytes a second. Nothing behind the
//! edit is serialized or validated again.
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

/// Remembers the canonical bytes of one project and how far they were hashed.
///
/// Give it the same project every time and report each change first:
/// `replaced` with what an edit replaced, or `objects_changed`,
/// `objects_changed_from` and `member_changed` for the parts, or `reset` when
/// the project is another one. The result is bit-identical to
/// `Project::state_hash`. It does not validate.
///
/// It holds a copy of the canonical bytes, about as much memory again as the
/// objects themselves take.
#[derive(Debug, Clone, Default)]
pub struct StateHasher {
    known: Option<Known>,
    /// Objects created, changed or removed since the last hash.
    objects: BTreeSet<Id>,
    /// Every object from this ID on may have changed since the last hash.
    objects_from: Option<Id>,
    /// Members other than `objects` changed since the last hash, by position.
    members: Vec<usize>,
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
}

impl Page {
    fn start(&self, index: usize) -> usize {
        if index == 0 {
            0
        } else {
            self.ends[index - 1] as usize
        }
    }
    /// The ID text of the entry at `index`.
    fn id(&self, index: usize) -> &[u8] {
        let start = self.start(index) + ID_AT;
        &self.bytes[start..start + ID_LEN]
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
    fn splice(&mut self, range: std::ops::Range<usize>, entry: &[u8]) -> Result<(), ModelError> {
        let (from, to) = (self.start(range.start), self.start(range.end));
        self.bytes.splice(from..to, entry.iter().copied());
        let end = u32::try_from(from + entry.len())
            .map_err(|_| ModelError::Invalid("canonical object size"))?;
        let removed = to - from;
        let inserted = if entry.is_empty() { None } else { Some(end) };
        self.ends.splice(range.clone(), inserted);
        let next = range.start + usize::from(!entry.is_empty());
        for later in &mut self.ends[next..] {
            *later = u32::try_from(*later as usize - removed + entry.len())
                .map_err(|_| ModelError::Invalid("canonical object size"))?;
        }
        Ok(())
    }
    /// Cut the page in two at the entry nearest its middle.
    fn split(&mut self) -> Option<Self> {
        let half = self.bytes.len() / 2;
        let at = self.ends.partition_point(|end| (*end as usize) <= half);
        if at == 0 || at >= self.ends.len() {
            return None;
        }
        let cut = self.ends[at - 1];
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
        })
    }
}

impl StateHasher {
    /// An edit replaced these entities; see `Replaced`.
    pub fn replaced(&mut self, replaced: &Replaced) {
        self.objects_changed(replaced.objects.keys());
        for (name, touched) in [
            ("documents", !replaced.documents.is_empty()),
            ("assets", !replaced.assets.is_empty()),
            ("layers", !replaced.layers.is_empty()),
            ("groups", !replaced.groups.is_empty()),
            ("instructions", !replaced.instructions.is_empty()),
            (
                "semantic_snapshots",
                !replaced.semantic_snapshots.is_empty(),
            ),
            ("results", !replaced.results.is_empty()),
            ("mask_versions", !replaced.mask_versions.is_empty()),
        ] {
            if touched {
                self.member_changed(name);
            }
        }
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
            Some(position) if !self.members.contains(&position) => self.members.push(position),
            Some(_) => {}
            // Not a member this hasher can refresh on its own.
            None => self.reset(),
        }
    }

    /// The project was replaced, or changed in a way not reported.
    pub fn reset(&mut self) {
        self.known = None;
        self.objects.clear();
        self.objects_from = None;
        self.members.clear();
    }

    /// The state hash of `project` and the length of its canonical bytes.
    pub fn hash(&mut self, project: &Project) -> Result<(StateHash, usize), ModelError> {
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
        let known = match &mut self.known {
            Some(known) => {
                for position in members {
                    known.refresh_member(project, position)?;
                }
                // Everything from `from` on is read again; single objects
                // before it are brought up to date one by one.
                for id in &objects {
                    if from.as_ref().is_none_or(|from| id < from) {
                        known.sync(project, id)?;
                    }
                }
                if let Some(from) = &from {
                    known.truncate(from);
                    known.extend(project, Some(from))?;
                }
                known
            }
            None => {
                let mut known = Known {
                    outline: Outline::of(project)?,
                    pages: Vec::new(),
                    fresh: 0,
                    end: None,
                    bytes: 0,
                };
                known.extend(project, None)?;
                self.known.insert(known)
            }
        };
        Ok(known.finish())
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
    fn sync(&mut self, project: &Project, id: &Id) -> Result<(), ModelError> {
        let mut entry = Vec::new();
        if let Some(object) = project.objects.get(id) {
            write_entry(&mut entry, id, object)?;
        }
        if self.pages.is_empty() {
            return self.append(&entry);
        }
        let index = self.page_of(id);
        match self.pages[index].find(id) {
            Ok(position) => {
                self.bytes -=
                    self.pages[index].start(position + 1) - self.pages[index].start(position);
                self.bytes += entry.len();
                self.pages[index].splice(position..position + 1, &entry)?;
                self.changed(index);
                if self.pages[index].ends.is_empty() {
                    self.pages.remove(index);
                    self.fresh = self.fresh.min(index);
                }
            }
            Err(_) if entry.is_empty() => {}
            Err(position) => {
                let last = index + 1 == self.pages.len();
                if last && position == self.pages[index].ends.len() {
                    return self.append(&entry);
                }
                self.bytes += entry.len();
                self.pages[index].splice(position..position, &entry)?;
                self.changed(index);
                if self.pages[index].bytes.len() > PAGE_SPLIT
                    && let Some(second) = self.pages[index].split()
                {
                    self.pages.insert(index + 1, second);
                }
            }
        }
        Ok(())
    }

    /// Add an entry behind every other. The state behind the last entry, when
    /// it is current, takes the new bytes and stays current.
    fn append(&mut self, entry: &[u8]) -> Result<(), ModelError> {
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
            });
        }
        let index = self.pages.len() - 1;
        let page = &mut self.pages[index];
        page.bytes.extend_from_slice(entry);
        page.ends.push(
            u32::try_from(page.bytes.len())
                .map_err(|_| ModelError::Invalid("canonical object size"))?,
        );
        self.bytes += entry.len();
        match &mut self.end {
            Some(end) if self.fresh == self.pages.len() => {
                end.update(if first { &entry[1..] } else { entry });
            }
            _ => self.changed(index),
        }
        Ok(())
    }

    /// Forget every entry from `from` on.
    fn truncate(&mut self, from: &Id) {
        if self.pages.is_empty() {
            return;
        }
        let index = self.page_of(from);
        let position = match self.pages[index].find(from) {
            Ok(position) | Err(position) => position,
        };
        let cut = self.pages[index].start(position);
        let removed: usize = self.pages[index].bytes.len() - cut
            + self.pages[index + 1..]
                .iter()
                .map(|page| page.bytes.len())
                .sum::<usize>();
        if removed == 0 {
            return;
        }
        self.bytes -= removed;
        self.pages.truncate(index + 1);
        self.pages[index].bytes.truncate(cut);
        self.pages[index].ends.truncate(position);
        self.changed(index);
        if position == 0 {
            self.pages.pop();
            self.fresh = self.fresh.min(index);
        }
    }

    /// Append the entries of the project's objects from `from` on.
    fn extend(&mut self, project: &Project, from: Option<&Id>) -> Result<(), ModelError> {
        let mut entry = Vec::new();
        let objects = match from {
            Some(from) => project.objects.range::<Id, _>(from..),
            None => project.objects.range::<Id, _>(..),
        };
        for (id, object) in objects {
            entry.clear();
            write_entry(&mut entry, id, object)?;
            self.append(&entry)?;
        }
        Ok(())
    }

    /// Serialize one member again; hashing restarts in front of it.
    fn refresh_member(&mut self, project: &Project, position: usize) -> Result<(), ModelError> {
        if self.outline.refresh(project, position)? {
            self.fresh = 0;
            self.end = None;
        }
        Ok(())
    }

    /// Hash from the first byte that is not hashed yet to the end.
    fn finish(&mut self) -> (StateHash, usize) {
        let end = match self.end.take() {
            Some(end) => end,
            None => {
                let mut hasher = if self.fresh == 0 || self.pages.is_empty() {
                    self.fresh = 0;
                    let mut hasher = blake3::Hasher::new();
                    hasher.update(DOMAIN);
                    hasher.update(&self.outline.head);
                    hasher
                } else {
                    self.fresh -= 1;
                    self.pages[self.fresh].before.clone()
                };
                for (index, page) in self.pages.iter_mut().enumerate().skip(self.fresh) {
                    page.before = hasher.clone();
                    hasher.update(if index == 0 {
                        &page.bytes[1..]
                    } else {
                        &page.bytes
                    });
                }
                self.fresh = self.pages.len();
                hasher
            }
        };
        let mut hasher = end.clone();
        self.end = Some(end);
        hasher.update(&self.outline.tail);
        // The comma in front of the first entry is not part of the bytes.
        let length =
            self.outline.head.len() + self.bytes.saturating_sub(1) + self.outline.tail.len();
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
/// member by member so that one changed member is serialized alone.
#[derive(Debug, Clone)]
struct Outline {
    /// `"name":value` of every member in the order serde_json writes them;
    /// for `objects` only `"objects":{`.
    members: Vec<(String, Vec<u8>)>,
    /// The position of `objects` among them.
    objects: usize,
    /// Everything in front of the first object entry.
    head: Vec<u8>,
    /// Everything behind the last one.
    tail: Vec<u8>,
}

impl Outline {
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
        normalize(&mut value);
        let serde_json::Value::Object(values) = value else {
            return Err(ModelError::Invalid("canonical project outline"));
        };
        let mut members = Vec::with_capacity(values.len());
        let mut objects = None;
        for (index, (name, member)) in values.iter().enumerate() {
            let mut bytes = Vec::new();
            if name == "objects" {
                objects = Some(index);
                write_name(&mut bytes, name)?;
                bytes.push(b'{');
            } else {
                write_member(&mut bytes, name, member)?;
            }
            members.push((name.clone(), bytes));
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
            .position(|(member, _)| member == name)
            .filter(|position| *position != self.objects)
    }

    /// Serialize the member at `position` again. True when it is written in
    /// front of the objects, so that every byte behind it has moved.
    fn refresh(&mut self, project: &Project, position: usize) -> Result<bool, ModelError> {
        let (name, bytes) = self
            .members
            .get_mut(position)
            .ok_or(ModelError::Invalid("canonical project outline"))?;
        let mut value = match name.as_str() {
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
        normalize(&mut value);
        bytes.clear();
        write_member(bytes, name, &value)?;
        let before = position < self.objects;
        self.join(before, !before);
        Ok(before)
    }

    /// Write the members exactly as serde_json writes the whole value, and
    /// split inside the braces of `objects`.
    fn join(&mut self, head: bool, tail: bool) {
        if head {
            self.head.clear();
            self.head.push(b'{');
            for (index, (_, bytes)) in self.members[..=self.objects].iter().enumerate() {
                if index > 0 {
                    self.head.push(b',');
                }
                self.head.extend_from_slice(bytes);
            }
        }
        if tail {
            self.tail.clear();
            self.tail.push(b'}');
            for (_, bytes) in &self.members[self.objects + 1..] {
                self.tail.push(b',');
                self.tail.extend_from_slice(bytes);
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

/// One `,"id":{object}` member of `objects`.
fn write_entry(out: &mut Vec<u8>, id: &Id, object: &Object) -> Result<(), ModelError> {
    out.push(b',');
    serde_json::to_writer(&mut *out, id)?;
    out.push(b':');
    let mut value = serde_json::to_value(object)?;
    normalize(&mut value);
    serde_json::to_writer(&mut *out, &value)?;
    Ok(())
}
