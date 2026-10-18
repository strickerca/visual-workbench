//! Device-local camera/bookmark settings. These never enter the canonical
//! project, operation history, checkpoints or network replication. Archive
//! export removes the entire exact namespace before its final VACUUM.
use crate::{ProjectStore, StoreError, migrations};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use vw_model::{DeviceId, Id};

pub const LOCAL_VIEW_PREFIX: &str = "vw.local-view.v1.";
pub const LOCAL_VIEW_MAX_DOCUMENTS: usize = 128;
pub const LOCAL_VIEW_MAX_BOOKMARKS: usize = 32;
pub const LOCAL_VIEW_MAX_NAME_BYTES: usize = 256;
pub const LOCAL_VIEW_MAX_VALUE_BYTES: usize = 16 * 1024;
pub const LOCAL_VIEW_MAX_ROWS: usize = 2048;

#[derive(Debug, thiserror::Error)]
pub enum LocalViewError {
    #[error("invalid local view")]
    Invalid,
    #[error("local view capacity exceeded")]
    Limit,
    #[error("local view changed; reload before retrying")]
    Conflict,
    #[error("local view operation was cancelled")]
    Cancelled,
    #[error("local view data is corrupt")]
    Corrupt,
    #[error("local view storage failed")]
    Storage(#[from] StoreError),
}
impl From<rusqlite::Error> for LocalViewError {
    fn from(error: rusqlite::Error) -> Self { Self::Storage(StoreError::Database(error)) }
}
type Result<T> = std::result::Result<T, LocalViewError>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalCamera {
    pub center_x: f64,
    pub center_y: f64,
    pub scale: f64,
    pub rotation: f64,
    pub viewport_width: f64,
    pub viewport_height: f64,
}
impl LocalCamera {
    pub fn validate(self) -> Result<()> {
        if ![self.center_x,self.center_y,self.scale,self.rotation,self.viewport_width,self.viewport_height].iter().all(|v|v.is_finite())
            || self.center_x.abs()>1e12 || self.center_y.abs()>1e12
            || !(1e-6..=1e6).contains(&self.scale) || self.rotation.abs()>1e6
            || !(1.0..=65_536.0).contains(&self.viewport_width) || !(1.0..=65_536.0).contains(&self.viewport_height) {
            return Err(LocalViewError::Invalid);
        }
        Ok(())
    }
    fn bits(self) -> CameraBits { CameraBits([self.center_x.to_bits(),self.center_y.to_bits(),self.scale.to_bits(),
        self.rotation.to_bits(),self.viewport_width.to_bits(),self.viewport_height.to_bits()]) }
}
// IEEE bits, not JSON decimal floats: even signed zero and difficult binary64
// values round-trip exactly, independent of serde_json float parser features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct CameraBits([u64;6]);
impl CameraBits {
    fn camera(self) -> Result<LocalCamera> {
        let c=LocalCamera{center_x:f64::from_bits(self.0[0]),center_y:f64::from_bits(self.0[1]),scale:f64::from_bits(self.0[2]),
            rotation:f64::from_bits(self.0[3]),viewport_width:f64::from_bits(self.0[4]),viewport_height:f64::from_bits(self.0[5])};
        c.validate()?;Ok(c)
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct LocalBookmark { pub id:Id, pub name:String, pub camera:LocalCamera }
#[derive(Debug, Clone, PartialEq)]
pub struct LocalViewSnapshot { pub revision:u64, pub camera:Option<LocalCamera>, pub bookmarks:Vec<LocalBookmark> }
#[derive(Debug, Clone)]
pub enum LocalViewChange {
    Remember(LocalCamera),
    AddBookmark { id:Id, name:String, camera:LocalCamera },
    RenameBookmark { id:Id, name:String },
    DeleteBookmark { id:Id },
}
impl LocalViewChange {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Remember(camera)=>camera.validate(),
            Self::AddBookmark{name,camera,..}=>{valid_name(name)?;camera.validate()},
            Self::RenameBookmark{name,..}=>valid_name(name),
            Self::DeleteBookmark{..}=>Ok(()),
        }
    }
    fn digest(&self, expected_revision:u64)->Result<String> {
        #[derive(Serialize)]
        enum Command<'a> { Remember(CameraBits), Add(&'a Id,&'a str,CameraBits), Rename(&'a Id,&'a str), Delete(&'a Id) }
        let value=match self {
            Self::Remember(camera)=>Command::Remember(camera.bits()),
            Self::AddBookmark{id,name,camera}=>Command::Add(id,name,camera.bits()),
            Self::RenameBookmark{id,name}=>Command::Rename(id,name),
            Self::DeleteBookmark{id}=>Command::Delete(id),
        };
        // The CAS precondition is part of the immutable request too. A caller
        // may retry a lost reply, but cannot reuse its operation identity with
        // a different expected revision even when the mutation bytes match.
        let bytes=serde_json::to_vec(&("vw.local-view.change.v1",expected_revision,&value)).map_err(|_|LocalViewError::Invalid)?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }
}
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Bookmark { id:Id, name:String, camera:CameraBits }
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt { operation:Id, digest:String }
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredView { version:u32, revision:u64, camera:Option<CameraBits>, bookmarks:Vec<Bookmark>, receipt:Option<Receipt> }
impl StoredView {
    fn empty()->Self { Self{version:1,revision:0,camera:None,bookmarks:Vec::new(),receipt:None} }
    fn validate(&self)->Result<()> {
        if self.version!=1 || self.revision>i64::MAX as u64 || self.bookmarks.len()>LOCAL_VIEW_MAX_BOOKMARKS
            || (self.revision==0)!=(self.receipt.is_none()) {return Err(LocalViewError::Corrupt)}
        if let Some(camera)=self.camera {camera.camera().map_err(|_|LocalViewError::Corrupt)?;}
        for (i,bookmark) in self.bookmarks.iter().enumerate() {
            valid_name(&bookmark.name).map_err(|_|LocalViewError::Corrupt)?;
            bookmark.camera.camera().map_err(|_|LocalViewError::Corrupt)?;
            if self.bookmarks[..i].iter().any(|b|b.id==bookmark.id||b.name==bookmark.name){return Err(LocalViewError::Corrupt)}
        }
        if self.receipt.as_ref().is_some_and(|r|r.digest.len()!=64||!r.digest.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b))) {
            return Err(LocalViewError::Corrupt);
        }
        Ok(())
    }
    fn snapshot(&self)->Result<LocalViewSnapshot> {
        self.validate()?;
        Ok(LocalViewSnapshot{revision:self.revision,camera:self.camera.map(CameraBits::camera).transpose()?,
            bookmarks:self.bookmarks.iter().map(|b|Ok(LocalBookmark{id:b.id.clone(),name:b.name.clone(),camera:b.camera.camera()?})).collect::<Result<_>>()?})
    }
}
fn valid_name(name:&str)->Result<()> {
    if name.is_empty()||name.len()>LOCAL_VIEW_MAX_NAME_BYTES||name.trim()!=name||name.chars().any(char::is_control){Err(LocalViewError::Invalid)}else{Ok(())}
}
fn key(device:&DeviceId,document:&Id)->String {format!("{LOCAL_VIEW_PREFIX}{}.{}",device.as_str(),document.as_str())}
fn load(connection:&Connection,key:&str)->Result<(StoredView,Option<String>)> {
    let admission:Option<(String,i64)>=connection.query_row("SELECT typeof(value),length(CAST(value AS BLOB)) FROM settings WHERE key=?1",[key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    match admission {
        None=>Ok((StoredView::empty(),None)),
        Some((kind,bytes))=>{
            if kind!="text" || bytes<1 || bytes>LOCAL_VIEW_MAX_VALUE_BYTES as i64 {return Err(LocalViewError::Corrupt)}
            let raw:String=connection.query_row("SELECT value FROM settings WHERE key=?1",[key],|r|r.get(0))?;
            // Recheck the owned value too: callers keep this pair of reads in a
            // transaction, but this guard prevents accidental unbounded reuse.
            if raw.len()>LOCAL_VIEW_MAX_VALUE_BYTES{return Err(LocalViewError::Corrupt)}
            let value:StoredView=serde_json::from_str(&raw).map_err(|_|LocalViewError::Corrupt)?;
            value.validate()?;
            if value.revision==0{return Err(LocalViewError::Corrupt)}
            Ok((value,Some(raw)))
        }
    }
}
fn admit_schema(connection:&Connection)->Result<()> {
    // verify_schema materializes every admitted schema row. Cap their aggregate
    // UTF-8 bytes before that allocation, including all names/SQL, so the local
    // facade's 1MiB envelope cannot expand with an unexpected live trigger.
    let (count,bytes):(i64,i64)=connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(length(CAST(type AS BLOB))+length(CAST(name AS BLOB))+length(CAST(tbl_name AS BLOB))+length(CAST(COALESCE(sql,'') AS BLOB))),0) FROM (SELECT type,name,tbl_name,sql FROM sqlite_master LIMIT 257)",
        [],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if count>256||!(0..=65_536).contains(&bytes){return Err(LocalViewError::Limit)}
    migrations::verify_schema(connection,2)?;
    Ok(())
}
impl ProjectStore {
    /// Reads only one bounded local settings row. Installation identity is an
    /// explicit namespace, never inferred from the project's original author.
    pub fn local_view(&mut self,device:&DeviceId,document:&Id)->Result<LocalViewSnapshot> {
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
        let (view,_)=load(&tx,&key(device,document))?;
        let snapshot=view.snapshot()?;tx.commit()?;Ok(snapshot)
    }
    /// Atomically persists local state without a canonical transaction. The
    /// latest operation ID/digest permits an exact ambiguous-result retry; any
    /// intervening writer forces a reload. A changed retry is never accepted.
    /// `cancelled` is inspected before each publication/commit, never after a
    /// successful commit has become durable. No files or unknown settings move.
    pub fn change_local_view(&mut self,device:&DeviceId,document:&Id,expected_revision:u64,operation:&Id,
        change:LocalViewChange,cancelled:impl Fn()->bool)->Result<LocalViewSnapshot> {
        change.validate()?;
        if expected_revision>i64::MAX as u64{return Err(LocalViewError::Invalid)}
        if cancelled(){return Err(LocalViewError::Cancelled)}
        let digest=change.digest(expected_revision)?;let key=key(device,document);
        let tx=self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        admit_schema(&tx)?;
        let (mut view,old)=load(&tx,&key)?;
        if let Some(receipt)=&view.receipt && receipt.operation==*operation {
            if receipt.digest!=digest{return Err(LocalViewError::Conflict)}
            let snapshot=view.snapshot()?;tx.commit()?;return Ok(snapshot);
        }
        if view.revision!=expected_revision{return Err(LocalViewError::Conflict)}
        if old.is_none() {
            let device_pattern=format!("{LOCAL_VIEW_PREFIX}{}.*",device.as_str());
            let device_count:i64=tx.query_row("SELECT COUNT(*) FROM (SELECT 1 FROM settings WHERE key GLOB ?1 LIMIT 129)",[device_pattern],|r|r.get(0))?;
            let total:i64=tx.query_row("SELECT COUNT(*) FROM (SELECT 1 FROM settings WHERE key GLOB 'vw.local-view.v1.*' LIMIT 2049)",[],|r|r.get(0))?;
            if device_count>=LOCAL_VIEW_MAX_DOCUMENTS as i64||total>=LOCAL_VIEW_MAX_ROWS as i64{return Err(LocalViewError::Limit)}
        }
        match change {
            LocalViewChange::Remember(camera)=>view.camera=Some(camera.bits()),
            LocalViewChange::AddBookmark{id,name,camera}=>{
                if view.bookmarks.len()>=LOCAL_VIEW_MAX_BOOKMARKS{return Err(LocalViewError::Limit)}
                if view.bookmarks.iter().any(|b|b.id==id||b.name==name){return Err(LocalViewError::Conflict)}
                view.bookmarks.push(Bookmark{id,name,camera:camera.bits()});
            },
            LocalViewChange::RenameBookmark{id,name}=>{
                if view.bookmarks.iter().any(|b|b.id!=id&&b.name==name){return Err(LocalViewError::Conflict)}
                let bookmark=view.bookmarks.iter_mut().find(|b|b.id==id).ok_or(LocalViewError::Conflict)?;
                bookmark.name=name;
            },
            LocalViewChange::DeleteBookmark{id}=>{
                let index=view.bookmarks.iter().position(|b|b.id==id).ok_or(LocalViewError::Conflict)?;
                view.bookmarks.remove(index);
            },
        }
        view.revision=view.revision.checked_add(1).filter(|v|*v<=i64::MAX as u64).ok_or(LocalViewError::Limit)?;
        view.receipt=Some(Receipt{operation:operation.clone(),digest});
        view.validate()?;
        let encoded=serde_json::to_string(&view).map_err(|_|LocalViewError::Invalid)?;
        if encoded.len()>LOCAL_VIEW_MAX_VALUE_BYTES{return Err(LocalViewError::Limit)}
        if cancelled(){return Err(LocalViewError::Cancelled)}
        let changed=if let Some(old)=old {
            tx.execute("UPDATE settings SET value=?1 WHERE key=?2 AND value=?3",params![encoded,key,old])?
        } else {tx.execute("INSERT INTO settings(key,value) VALUES (?1,?2)",params![key,encoded])?};
        if changed!=1{return Err(LocalViewError::Conflict)}
        let (persisted,raw)=load(&tx,&key)?;
        if raw.as_deref()!=Some(encoded.as_str()){return Err(LocalViewError::Corrupt)}
        let result=persisted.snapshot()?;
        if cancelled(){return Err(LocalViewError::Cancelled)}
        tx.commit()?;
        Ok(result)
    }
}
