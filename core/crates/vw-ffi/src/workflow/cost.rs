//! What a value costs in memory as a `serde_json::Value` tree and as compact
//! JSON bytes, counted from its serialization without building either.
//!
//! The workflows hash the visible project (`Project::state_hash` builds a
//! `Value` tree of it and then its bytes) and record every entity an edit
//! touches as `Value`s. Counting runs the value's own `Serialize` code through
//! `serde_json` with a formatter that only tallies, so the shape counted is
//! the shape `serde_json::to_value` builds: one `Value` per scalar or string,
//! one `Vec` per array and one `BTreeMap` per object or struct.
//!
//! The same count bounds a clone of the value itself, for the model's types:
//! a scalar in a sequence takes the width of its type in its `Vec`, and
//! every other value at most what its `Value` would. A struct's own fields, `None`
//! messages and enum variants included, fit in the map nodes counted for it.
use serde::Serialize;
use serde_json::ser::{CharEscape, CompactFormatter, Formatter, Serializer};
use std::io;

/// One `serde_json::Value` on a 64-bit target (no `arbitrary_precision`).
const VALUE: u64 = 32;
/// One node of the `BTreeMap<String, Value>` a `Value` object holds: eleven
/// 24-byte keys and eleven values, the parent link, index and length (632
/// bytes in a leaf), and twelve child links in an internal node.
const MAP_NODE: u64 = 728;
/// Fewest entries a node other than the root holds: nodes are only ever
/// split while a map is built, and a split leaves at least this many.
const MAP_MIN_ENTRIES: u64 = 5;

/// The two representations of one value, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct JsonCost {
    /// Length of its compact JSON as a `Value` writes it: every 32-bit float
    /// as the longer 64-bit number the `Value` holds, zeros not shortened.
    pub text: u64,
    /// Heap bytes of the `Value` tree: array slots, map nodes and string
    /// bytes. The root `Value` itself lives on the stack.
    pub tree: u64,
    /// Heap bytes of a clone of the value: the tree with every scalar in a
    /// sequence at the width of its type in an exactly sized `Vec`.
    pub copy: u64,
    /// For a project: the text and tree of its `objects` member alone.
    pub objects_text: u64,
    pub objects_tree: u64,
    /// For a project: the hashed cost of its largest object.
    pub largest_object: u64,
}
impl JsonCost {
    /// What `Project::state_hash` allocates for a project of this cost: the
    /// tree, and while it is alive `to_vec` of it, whose doubling holds an
    /// old and a new buffer of together at most three times the text; then,
    /// with the tree gone, those bytes and a copy behind the hash domain.
    pub fn hashed(self) -> u64 {
        self.tree
            .saturating_add(self.text.saturating_mul(3))
            .saturating_add(256)
    }
    /// Text of a project's members other than `objects`.
    pub fn shell_text(self) -> u64 {
        self.text.saturating_sub(self.objects_text)
    }
    /// What a new `vw_model::StateHasher` allocates to hash a project of
    /// this cost. The members other than `objects` become one `Value` tree,
    /// written to their bytes, which are kept three times (per entity, per
    /// member, and joined in front of and behind `objects`) and joined again
    /// by doubling. Every object becomes a `Value` and its bytes, one at a
    /// time, and is kept in pages filled by doubling, of at most 512 KiB.
    pub fn hasher(self) -> u64 {
        self.tree
            .saturating_sub(self.objects_tree)
            .saturating_add(self.shell_text().saturating_mul(5))
            .saturating_add(self.objects_text.saturating_mul(2))
            .saturating_add(self.largest_object)
            .saturating_add(1024 * 1024)
    }
}

/// Count `value`, stopping once its `hashed` cost passes `limit`.
/// `None` means it passed; nothing is retained either way.
pub(crate) fn measure<T: Serialize + ?Sized>(
    value: &T,
    limit: u64,
) -> io::Result<Option<JsonCost>> {
    let mut tally = Tally {
        cost: JsonCost::default(),
        frames: Vec::new(),
        string_start: None,
        limit,
        exceeded: false,
        key: Vec::new(),
        member: None,
        entity: None,
    };
    // Every byte goes through the formatter, which writes to its own count;
    // anything written past it would still be counted here.
    let mut stray = Count(0);
    let result = value.serialize(&mut Serializer::with_formatter(&mut stray, &mut tally));
    if tally.exceeded {
        return Ok(None);
    }
    result.map_err(io::Error::other)?;
    let cost = JsonCost {
        text: tally.cost.text.saturating_add(stray.0),
        ..tally.cost
    };
    Ok((cost.hashed() <= limit).then_some(cost))
}

struct Count(u64);
impl io::Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len() as u64);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

enum Frame {
    /// Elements, how many of them were scalars, and their widths together.
    Array(u64, u64, u64),
    Object(u64),
}

struct Tally {
    cost: JsonCost,
    frames: Vec<Frame>,
    /// Text length where the open string's contents began.
    string_start: Option<u64>,
    limit: u64,
    exceeded: bool,
    /// The key of the member being written, at the top level.
    key: Vec<u8>,
    /// Text and tree where the `objects` member's value began.
    member: Option<(u64, u64)>,
    /// Text and tree where an object began.
    entity: Option<(u64, u64)>,
}
impl Tally {
    fn text(&mut self, length: usize) -> io::Result<()> {
        self.cost.text = self.cost.text.saturating_add(length as u64);
        self.check()
    }
    fn tree(&mut self, bytes: u64) -> io::Result<()> {
        self.cost.tree = self.cost.tree.saturating_add(bytes);
        self.cost.copy = self.cost.copy.saturating_add(bytes);
        self.check()
    }
    /// A scalar of `width` bytes: in a sequence, not as the text of a key.
    fn scalar(&mut self, width: u64) {
        if self.string_start.is_none()
            && let Some(Frame::Array(_, scalars, widths)) = self.frames.last_mut()
        {
            *scalars += 1;
            *widths += width;
        }
    }
    fn check(&mut self) -> io::Result<()> {
        if self.cost.hashed() > self.limit {
            self.exceeded = true;
            return Err(io::Error::other("workspace admission bound"));
        }
        Ok(())
    }
    /// Write through the compact formatter, counting what it writes.
    fn compact(
        &mut self,
        width: u64,
        write: impl FnOnce(&mut CompactFormatter, &mut Count) -> io::Result<()>,
    ) -> io::Result<()> {
        self.scalar(width);
        let mut count = Count(0);
        write(&mut CompactFormatter, &mut count)?;
        self.cost.text = self.cost.text.saturating_add(count.0);
        self.check()
    }
}

/// A `Vec<Value>` built from a sequence: exactly its length when serde knew
/// it, or grown from empty by doubling, which leaves at most the next power
/// of two and never fewer than four slots.
fn array_heap(count: u64) -> u64 {
    if count == 0 {
        0
    } else {
        count
            .max(4)
            .checked_next_power_of_two()
            .unwrap_or(u64::MAX)
            .saturating_mul(VALUE)
    }
}
/// A `BTreeMap<String, Value>` with `count` entries.
fn map_heap(count: u64) -> u64 {
    if count == 0 {
        0
    } else {
        (1 + (count - 1) / MAP_MIN_ENTRIES).saturating_mul(MAP_NODE)
    }
}

macro_rules! scalars {
    ($($name:ident: $type:ty),* $(,)?) => {$(
        fn $name<W: ?Sized + io::Write>(&mut self, _: &mut W, value: $type) -> io::Result<()> {
            let width = std::mem::size_of::<$type>() as u64;
            self.compact(width, |formatter, count| formatter.$name(count, value))
        }
    )*};
}

impl Formatter for &mut Tally {
    scalars!(
        write_bool: bool,
        write_i8: i8,
        write_i16: i16,
        write_i32: i32,
        write_i64: i64,
        write_i128: i128,
        write_u8: u8,
        write_u16: u16,
        write_u32: u32,
        write_u64: u64,
        write_u128: u128,
        write_f64: f64,
    );
    fn write_null<W: ?Sized + io::Write>(&mut self, _: &mut W) -> io::Result<()> {
        self.scalar(8);
        self.text(4)
    }
    /// A `Value` holds a 32-bit float as a 64-bit one, which writes longer.
    fn write_f32<W: ?Sized + io::Write>(&mut self, _: &mut W, value: f32) -> io::Result<()> {
        self.compact(4, |formatter, count| {
            formatter.write_f64(count, f64::from(value))
        })
    }
    fn write_number_str<W: ?Sized + io::Write>(
        &mut self,
        _: &mut W,
        value: &str,
    ) -> io::Result<()> {
        self.scalar(8);
        self.text(value.len())
    }
    fn begin_string<W: ?Sized + io::Write>(&mut self, _: &mut W) -> io::Result<()> {
        self.text(1)?;
        self.string_start = Some(self.cost.text);
        Ok(())
    }
    /// A string's heap holds at most what its escaped text takes.
    fn end_string<W: ?Sized + io::Write>(&mut self, _: &mut W) -> io::Result<()> {
        let start = self.string_start.take().unwrap_or(self.cost.text);
        self.tree(self.cost.text.saturating_sub(start))?;
        self.text(1)
    }
    fn write_string_fragment<W: ?Sized + io::Write>(
        &mut self,
        _: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        // Only a top-level key is kept, and only as long as a member name.
        if self.frames.len() == 1 && self.key.len() < 32 {
            self.key.extend_from_slice(fragment.as_bytes());
        }
        self.text(fragment.len())
    }
    fn write_char_escape<W: ?Sized + io::Write>(
        &mut self,
        _: &mut W,
        escape: CharEscape,
    ) -> io::Result<()> {
        self.compact(0, |formatter, count| {
            formatter.write_char_escape(count, escape)
        })
    }
    fn begin_array<W: ?Sized + io::Write>(&mut self, _: &mut W) -> io::Result<()> {
        self.frames.push(Frame::Array(0, 0, 0));
        self.text(1)
    }
    fn end_array<W: ?Sized + io::Write>(&mut self, _: &mut W) -> io::Result<()> {
        if let Some(Frame::Array(count, scalars, widths)) = self.frames.pop() {
            let heap = array_heap(count);
            self.cost.tree = self.cost.tree.saturating_add(heap);
            // A cloned `Vec` is sized exactly: scalars take the width of
            // their type, anything else at most the slot its `Value` would.
            let copied = (count - scalars)
                .saturating_mul(VALUE)
                .saturating_add(widths);
            self.cost.copy = self.cost.copy.saturating_add(copied);
            self.check()?;
        }
        self.text(1)
    }
    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        _: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if let Some(Frame::Array(count, ..)) = self.frames.last_mut() {
            *count += 1;
        }
        self.text(usize::from(!first))
    }
    fn end_array_value<W: ?Sized + io::Write>(&mut self, _: &mut W) -> io::Result<()> {
        Ok(())
    }
    fn begin_object<W: ?Sized + io::Write>(&mut self, _: &mut W) -> io::Result<()> {
        self.frames.push(Frame::Object(0));
        self.text(1)
    }
    fn end_object<W: ?Sized + io::Write>(&mut self, _: &mut W) -> io::Result<()> {
        if let Some(Frame::Object(count)) = self.frames.pop() {
            self.tree(map_heap(count))?;
        }
        self.text(1)
    }
    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        _: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if let Some(Frame::Object(count)) = self.frames.last_mut() {
            *count += 1;
        }
        if self.frames.len() == 1 {
            self.key.clear();
        }
        self.text(usize::from(!first))
    }
    fn end_object_key<W: ?Sized + io::Write>(&mut self, _: &mut W) -> io::Result<()> {
        Ok(())
    }
    fn begin_object_value<W: ?Sized + io::Write>(&mut self, _: &mut W) -> io::Result<()> {
        self.text(1)?;
        let at = (self.cost.text, self.cost.tree);
        match self.frames.len() {
            1 if self.key == b"objects" => self.member = Some(at),
            2 if self.member.is_some() => self.entity = Some(at),
            _ => {}
        }
        Ok(())
    }
    fn end_object_value<W: ?Sized + io::Write>(&mut self, _: &mut W) -> io::Result<()> {
        match self.frames.len() {
            1 => {
                if let Some((text, tree)) = self.member.take() {
                    self.cost.objects_text = self.cost.text.saturating_sub(text);
                    self.cost.objects_tree = self.cost.tree.saturating_sub(tree);
                }
            }
            2 => {
                if let Some((text, tree)) = self.entity.take() {
                    let entity = JsonCost {
                        text: self.cost.text.saturating_sub(text),
                        tree: self.cost.tree.saturating_sub(tree),
                        ..JsonCost::default()
                    };
                    self.cost.largest_object = self.cost.largest_object.max(entity.hashed());
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn write_raw_fragment<W: ?Sized + io::Write>(
        &mut self,
        _: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        self.text(fragment.len())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::{Map, Value, json};

    /// Heap bytes of an actual `Value` tree, from what each node can hold.
    fn actual(value: &Value) -> u64 {
        match value {
            Value::String(text) => text.capacity() as u64,
            Value::Array(items) => {
                items.capacity() as u64 * VALUE + items.iter().map(actual).sum::<u64>()
            }
            Value::Object(map) => {
                map_heap(map.len() as u64)
                    + map
                        .iter()
                        .map(|(key, value)| key.capacity() as u64 + actual(value))
                        .sum::<u64>()
            }
            _ => 0,
        }
    }

    #[test]
    fn a_value_is_the_size_the_bound_assumes() {
        assert_eq!(std::mem::size_of::<Value>() as u64, VALUE);
        assert_eq!(std::mem::size_of::<String>(), 24);
    }

    #[test]
    fn text_is_the_length_a_value_writes_and_tree_covers_the_value() {
        #[derive(Serialize)]
        struct Sample {
            x: Vec<f64>,
            pressure: Vec<f32>,
            name: String,
            escaped: String,
            map: std::collections::BTreeMap<u32, bool>,
            bytes: Vec<u8>,
            nested: Vec<Vec<i64>>,
            nothing: Option<u8>,
        }
        let sample = Sample {
            x: (0..37).map(|n| f64::from(n) * 1.234_567_89 - 3.0).collect(),
            pressure: (0..37).map(|n| n as f32 / 37.0).collect(),
            name: "layer".into(),
            escaped: "a\"b\\c\nd\u{1}".into(),
            map: (0..23).map(|n| (n, n % 2 == 0)).collect(),
            bytes: vec![1, 2, 250],
            nested: vec![vec![], vec![1], vec![-1, 2, 3, 4, 5]],
            nothing: None,
        };
        let cost = measure(&sample, u64::MAX).unwrap().unwrap();
        let value = serde_json::to_value(&sample).unwrap();
        let text = serde_json::to_vec(&value).unwrap();
        assert_eq!(cost.text, text.len() as u64);
        assert!(
            cost.tree >= actual(&value),
            "{} < {}",
            cost.tree,
            actual(&value)
        );
        // Not wildly above either: arrays are at most a power of two over.
        assert!(cost.tree <= 2 * actual(&value) + 4096);
    }

    #[test]
    fn maps_count_every_node_a_large_object_needs() {
        let map: Map<String, Value> = (0..1000).map(|n| (n.to_string(), json!(n))).collect();
        let value = Value::Object(map);
        let cost = measure(&value, u64::MAX).unwrap().unwrap();
        // What `to_value` builds of it, with keys as long as their text.
        let rebuilt = serde_json::to_value(&value).unwrap();
        assert!(cost.tree >= actual(&rebuilt));
    }

    #[test]
    fn a_project_tells_its_objects_and_largest_object_apart() {
        #[derive(Serialize)]
        struct Page {
            assets: std::collections::BTreeMap<String, Vec<u8>>,
            objects: std::collections::BTreeMap<String, Vec<f64>>,
            title: String,
        }
        let page = Page {
            assets: [("a".to_string(), vec![7u8; 5000])].into(),
            objects: (0..20)
                .map(|n| (format!("{n:02}"), vec![1.25; 10 + n]))
                .collect(),
            title: "page".into(),
        };
        let cost = measure(&page, u64::MAX).unwrap().unwrap();
        let objects = serde_json::to_vec(&page.objects).unwrap();
        assert_eq!(cost.objects_text, objects.len() as u64);
        let largest = measure(&page.objects["19"], u64::MAX).unwrap().unwrap();
        assert_eq!(cost.largest_object, largest.hashed());
        // A copy holds the asset's bytes at one byte each, not as values.
        assert!(cost.copy < cost.tree);
        assert!(cost.copy >= 5000 + 20 * 8 * 10);
    }

    #[test]
    fn counting_stops_at_the_limit() {
        let big = vec![1.5f64; 100_000];
        let cost = measure(&big, u64::MAX).unwrap().unwrap();
        assert!(measure(&big, cost.hashed()).unwrap().is_some());
        assert!(measure(&big, cost.hashed() - 1).unwrap().is_none());
        assert!(measure(&big, 10).unwrap().is_none());
    }
}
