use crate::OpsError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use vw_model::Project;

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

pub(crate) fn between(before: &Project, after: &Project) -> Result<Vec<Change>, OpsError> {
    let before = serde_json::to_value(before)?;
    let after = serde_json::to_value(after)?;
    let mut changes = Vec::new();
    for collection in [
        "documents",
        "assets",
        "layers",
        "objects",
        "groups",
        "instructions",
        "semantic_snapshots",
        "results",
        "mask_versions",
    ] {
        let old = before
            .get(collection)
            .and_then(Value::as_object)
            .ok_or(OpsError::Invalid("journal collection"))?;
        let new = after
            .get(collection)
            .and_then(Value::as_object)
            .ok_or(OpsError::Invalid("journal collection"))?;
        let keys: BTreeSet<_> = old.keys().chain(new.keys()).collect();
        for key in keys {
            diff(
                Address {
                    collection: collection.into(),
                    entity: key.clone(),
                    path: Vec::new(),
                },
                old.get(key),
                new.get(key),
                &mut changes,
            );
        }
    }
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

pub(crate) fn apply(project: &Project, changes: &[Change]) -> Result<Project, OpsError> {
    let mut value = serde_json::to_value(project)?;
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
    let next: Project = serde_json::from_value(value)?;
    next.validate()?;
    Ok(next)
}
