use crate::{
    OpsError,
    draft::{COLLECTIONS, Draft, Originals, for_each_collection},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use vw_model::{AssetId, Id, Project};

/// Internal validated journal address: collection/entity/property. It never
/// crosses the wire as an arbitrary client-controlled patch.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct Address {
    pub collection: String,
    pub entity: String,
    pub path: Vec<String>,
}

impl Address {
    pub fn key(&self) -> String {
        std::iter::once(self.collection.as_str())
            .chain(std::iter::once(self.entity.as_str()))
            .chain(self.path.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("/")
    }
    pub fn overlaps(&self, other: &Self) -> bool {
        self.collection == other.collection
            && self.entity == other.entity
            && (self.path.starts_with(&other.path) || other.path.starts_with(&self.path))
    }
    pub fn property(&self) -> String {
        if self.path.is_empty() {
            "$entity".into()
        } else {
            self.path.join(".")
        }
    }
}

/// Before/after values are retained for inverse transactions, never reconstructed
/// from a lossy render or from the current edited value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Change {
    pub address: Address,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

impl Change {
    pub fn inverse(&self) -> Self {
        Self {
            address: self.address.clone(),
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

/// An entity key as the journal spells it.
trait Key: Ord + Sized {
    fn parse(text: &str) -> Option<Self>;
}
impl Key for Id {
    fn parse(text: &str) -> Option<Self> {
        Self::try_from(text.to_owned()).ok()
    }
}
impl Key for AssetId {
    fn parse(text: &str) -> Option<Self> {
        Self::try_from(text.to_owned()).ok()
    }
}

fn value_in<K: Key, V: Serialize>(
    entities: &BTreeMap<K, V>,
    entity: &str,
) -> Result<Option<Value>, OpsError> {
    Ok(K::parse(entity)
        .and_then(|key| entities.get(&key))
        .map(serde_json::to_value)
        .transpose()?)
}

/// One entity as it appears inside the project's JSON, if it exists.
fn current(project: &Project, collection: &str, entity: &str) -> Result<Option<Value>, OpsError> {
    match collection {
        "documents" => value_in(&project.documents, entity),
        "assets" => value_in(&project.assets, entity),
        "layers" => value_in(&project.layers, entity),
        "objects" => value_in(&project.objects, entity),
        "groups" => value_in(&project.groups, entity),
        "instructions" => value_in(&project.instructions, entity),
        "semantic_snapshots" => value_in(&project.semantic_snapshots, entity),
        "results" => value_in(&project.results, entity),
        "mask_versions" => value_in(&project.mask_versions, entity),
        _ => Ok(None),
    }
}

/// An entity's JSON before the edit, when the edit touched it at all.
fn original(
    before: &Originals,
    collection: &str,
    entity: &str,
) -> Result<Option<Option<Value>>, OpsError> {
    fn value_before<K: Key, V: Serialize>(
        originals: &BTreeMap<K, Option<V>>,
        entity: &str,
    ) -> Result<Option<Option<Value>>, OpsError> {
        let Some(original) = K::parse(entity).and_then(|key| originals.get(&key)) else {
            return Ok(None);
        };
        Ok(Some(
            original.as_ref().map(serde_json::to_value).transpose()?,
        ))
    }
    match collection {
        "documents" => value_before(&before.documents, entity),
        "assets" => value_before(&before.assets, entity),
        "layers" => value_before(&before.layers, entity),
        "objects" => value_before(&before.objects, entity),
        "groups" => value_before(&before.groups, entity),
        "instructions" => value_before(&before.instructions, entity),
        "semantic_snapshots" => value_before(&before.semantic_snapshots, entity),
        "results" => value_before(&before.results, entity),
        "mask_versions" => value_before(&before.mask_versions, entity),
        _ => Ok(None),
    }
}

/// The journal of an edit: how every entity it touched differs from what it
/// was. Entities are visited in collection order, then key order, which is the
/// order a comparison of the two whole projects would report them in.
pub(crate) fn between(before: &Originals, after: &Project) -> Result<Vec<Change>, OpsError> {
    let mut changes = Vec::new();
    macro_rules! collection {
        ($field:ident, $key:ty, $value:ty) => {
            for (id, original) in &before.$field {
                let old = original.as_ref().map(serde_json::to_value).transpose()?;
                let new = after.$field.get(id).map(serde_json::to_value).transpose()?;
                diff(
                    Address {
                        collection: stringify!($field).into(),
                        entity: id.to_string(),
                        path: Vec::new(),
                    },
                    old.as_ref(),
                    new.as_ref(),
                    &mut changes,
                );
            }
        };
    }
    for_each_collection!(collection);
    Ok(changes)
}

fn diff(
    address: Address,
    before: Option<&Value>,
    after: Option<&Value>,
    changes: &mut Vec<Change>,
) {
    if before == after {
        return;
    }
    // Affines and UUID references are coherent properties; they cannot be split
    // into independent coefficient/byte writes. Shape variant changes are also
    // atomic, while properties within an unchanged shape remain independent.
    let atomic = matches!(
        address.path.last().map(String::as_str),
        Some("transform" | "group_id" | "layer_id" | "document_id")
    );
    if let (Some(Value::Object(old)), Some(Value::Object(new))) = (before, after) {
        let variant_changed =
            address.path.last().is_some_and(|p| p == "shape") && old.keys().ne(new.keys());
        if !atomic && !variant_changed {
            let keys: BTreeSet<_> = old.keys().chain(new.keys()).collect();
            for key in keys {
                let mut child = address.clone();
                child.path.push(key.clone());
                diff(child, old.get(key), new.get(key), changes);
            }
            return;
        }
    }
    changes.push(Change {
        address,
        before: before.cloned(),
        after: after.cloned(),
    });
}

/// Entity JSON before and after an edit, converted when first asked for.
pub(crate) struct Values<'a> {
    before: &'a Originals,
    after: &'a Project,
    entities: BTreeMap<(String, String), (Option<Value>, Option<Value>)>,
}

impl<'a> Values<'a> {
    pub fn new(before: &'a Originals, after: &'a Project) -> Self {
        Self {
            before,
            after,
            entities: BTreeMap::new(),
        }
    }

    /// The values at `address` before and after; `None` where the entity or
    /// the property did not exist.
    pub fn at(&mut self, address: &Address) -> Result<(Option<&Value>, Option<&Value>), OpsError> {
        let key = (address.collection.clone(), address.entity.clone());
        if !self.entities.contains_key(&key) {
            let new = current(self.after, &address.collection, &address.entity)?;
            let old = match original(self.before, &address.collection, &address.entity)? {
                Some(old) => old,
                None => new.clone(),
            };
            self.entities.insert(key.clone(), (old, new));
        }
        let Some((old, new)) = self.entities.get(&key) else {
            return Ok((None, None));
        };
        Ok((descend(old, &address.path), descend(new, &address.path)))
    }
}

fn descend<'v>(entity: &'v Option<Value>, path: &[String]) -> Option<&'v Value> {
    let mut value = entity.as_ref()?;
    for part in path {
        value = value.get(part)?;
    }
    Some(value)
}

/// Apply journal changes to a draft, with the meaning they have on the whole
/// project's JSON: each change replaces or removes the value at its address,
/// and every entity a change addressed must then still be a valid entity.
/// Only those entities are converted to JSON and back.
pub(crate) fn apply(draft: &mut Draft<'_>, changes: &[Change]) -> Result<(), OpsError> {
    let mut staged = Map::new();
    // Members that are not collections refuse a change the way they always
    // did: nothing can be found inside them.
    for scalar in ["id", "title", "created_by", "schema_version"] {
        staged.insert(scalar.into(), Value::Null);
    }
    for collection in COLLECTIONS {
        staged.insert(collection.into(), Value::Object(Map::new()));
    }
    let mut addressed = BTreeSet::new();
    for change in changes {
        let address = &change.address;
        let Some(collection) = COLLECTIONS
            .iter()
            .position(|name| *name == address.collection)
        else {
            continue;
        };
        if addressed.insert((collection, address.entity.as_str()))
            && let Some(value) = current(draft.project(), &address.collection, &address.entity)?
            && let Some(Value::Object(entities)) = staged.get_mut(&address.collection)
        {
            entities.insert(address.entity.clone(), value);
        }
    }
    let mut value = Value::Object(staged);
    for change in changes {
        let mut cursor = value
            .get_mut(&change.address.collection)
            .ok_or(OpsError::Invalid("journal collection"))?;
        let mut path = vec![change.address.entity.as_str()];
        path.extend(change.address.path.iter().map(String::as_str));
        let (last, parents) = path.split_last().ok_or(OpsError::Invalid("journal path"))?;
        for parent in parents {
            cursor = cursor.get_mut(*parent).ok_or(OpsError::NotFound)?;
        }
        let object = cursor
            .as_object_mut()
            .ok_or(OpsError::Invalid("journal object"))?;
        match &change.after {
            Some(next) => {
                object.insert((*last).into(), next.clone());
            }
            None => {
                object.remove(*last);
            }
        }
    }
    // Collection order, then key order: the order the whole project would be
    // read back in, so the first malformed entity is the same one.
    for (collection, entity) in addressed {
        let staged = value
            .get_mut(COLLECTIONS[collection])
            .and_then(Value::as_object_mut)
            .and_then(|entities| entities.remove(entity));
        store(draft, COLLECTIONS[collection], entity, staged)?;
    }
    Ok(())
}

/// Put one staged entity back into the draft, or remove it when it is gone.
fn store(
    draft: &mut Draft<'_>,
    collection: &str,
    entity: &str,
    value: Option<Value>,
) -> Result<(), OpsError> {
    macro_rules! put {
        ($key:ty, $insert:ident, $remove:ident) => {
            match value {
                Some(value) => {
                    let key: $key = serde_json::from_value(Value::String(entity.to_owned()))?;
                    draft.$insert(key, serde_json::from_value(value)?);
                }
                None => {
                    if let Some(key) = <$key as Key>::parse(entity) {
                        draft.$remove(&key);
                    }
                }
            }
        };
    }
    match collection {
        "documents" => put!(Id, insert_document, remove_document),
        "assets" => put!(AssetId, insert_asset, remove_asset),
        "layers" => put!(Id, insert_layer, remove_layer),
        "objects" => put!(Id, insert_object, remove_object),
        "groups" => put!(Id, insert_group, remove_group),
        "instructions" => put!(Id, insert_instruction, remove_instruction),
        "semantic_snapshots" => put!(Id, insert_semantic_snapshot, remove_semantic_snapshot),
        "results" => put!(Id, insert_result, remove_result),
        "mask_versions" => put!(Id, insert_mask_versions, remove_mask_versions),
        _ => {}
    }
    Ok(())
}
