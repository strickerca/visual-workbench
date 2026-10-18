//! Bounded local settings, deliberately independent of canonical history/hash.
//! A caller's installation ID names the local view; it is not a trust assertion
//! and must not be inferred from the project's original author.
use crate::{Camera, Cancellation, ProjectSession, workflow::*};
use std::sync::{Arc, atomic::Ordering};
use vw_model::{DeviceId, Id};
use vw_store::{LocalCamera, LocalViewChange, LocalViewError};

// One <=16KiB row, <=32 names of <=256 UTF8 bytes, parsing/staging plus generated
// and common DTO representations fit this fixed envelope. This path borrows
// only project/document IDs: it never hashes, serializes or clones project/history.
const LOCAL_VIEW_WORK:u64=1024*1024;

#[derive(Debug, Clone, uniffi::Record)]
pub struct LocalViewsBinding { pub project_id:String, pub document_id:String, pub device_id:String }
#[derive(Debug, Clone, uniffi::Record)]
pub struct RegionBookmark { pub id:String, pub name:String, pub camera:Camera }
#[derive(Debug, Clone, uniffi::Record)]
pub struct LocalViewsSnapshot {
    pub binding:LocalViewsBinding,
    pub revision:u64,
    pub camera:Option<Camera>,
    pub bookmarks:Vec<RegionBookmark>,
}
#[derive(Debug, Clone, uniffi::Enum)]
pub enum LocalViewsMutation {
    Remember { camera:Camera },
    AddBookmark { id:String, name:String, camera:Camera },
    RenameBookmark { id:String, name:String },
    DeleteBookmark { id:String },
}
fn id(value:&str)->WorkflowResult<Id> {
    if value.len()!=36{return Err(WorkflowError::Invalid)}
    Ok(Id::try_from(value.to_owned())?)
}
fn camera(value:Camera)->WorkflowResult<LocalCamera> {
    let result=LocalCamera{center_x:value.center.x,center_y:value.center.y,scale:value.scale,rotation:value.rotation,
        viewport_width:value.viewport_width,viewport_height:value.viewport_height};
    result.validate().map_err(failure)?;Ok(result)
}
fn output(value:LocalCamera)->Camera { Camera{center:crate::Point{x:value.center_x,y:value.center_y},scale:value.scale,rotation:value.rotation,
    viewport_width:value.viewport_width,viewport_height:value.viewport_height} }
fn failure(error:LocalViewError)->WorkflowError {
    match error { LocalViewError::Invalid=>WorkflowError::Invalid,LocalViewError::Limit=>WorkflowError::Limit,
        LocalViewError::Conflict=>WorkflowError::Stale,LocalViewError::Cancelled=>WorkflowError::Cancelled,
        LocalViewError::Corrupt|LocalViewError::Storage(_)=>WorkflowError::Storage }
}
fn validate(binding:&LocalViewsBinding,budget:u64)->WorkflowResult<(Id,Id,DeviceId)> {
    if budget<LOCAL_VIEW_WORK{return Err(WorkflowError::Memory{estimated:LOCAL_VIEW_WORK,budget})}
    if binding.device_id.len()!=26{return Err(WorkflowError::Invalid)}
    Ok((id(&binding.project_id)?,id(&binding.document_id)?,DeviceId::try_from(binding.device_id.clone())?))
}
fn check_project(state:&crate::project::ProjectState,project:&Id,document:&Id)->WorkflowResult<()> {
    let value=state.project()?;
    if value.id!=*project{return Err(WorkflowError::Detached)}
    if !value.documents.contains_key(document){return Err(WorkflowError::Missing)}
    Ok(())
}
fn snapshot(binding:LocalViewsBinding,value:vw_store::LocalViewSnapshot)->LocalViewsSnapshot {
    LocalViewsSnapshot{binding,revision:value.revision,camera:value.camera.map(output),bookmarks:value.bookmarks.into_iter()
        .map(|b|RegionBookmark{id:b.id.to_string(),name:b.name,camera:output(b.camera)}).collect()}
}
impl LocalViewsMutation {
    fn owned(self)->WorkflowResult<LocalViewChange> {
        fn name(value:String)->WorkflowResult<String>{if value.is_empty()||value.len()>vw_store::LOCAL_VIEW_MAX_NAME_BYTES||value.trim()!=value||value.chars().any(char::is_control){Err(WorkflowError::Invalid)}else{Ok(value)}}
        Ok(match self {
            Self::Remember{camera:value}=>LocalViewChange::Remember(camera(value)?),
            Self::AddBookmark{id:value,name:title,camera:value_camera}=>LocalViewChange::AddBookmark{id:id(&value)?,name:name(title)?,camera:camera(value_camera)?},
            Self::RenameBookmark{id:value,name:title}=>LocalViewChange::RenameBookmark{id:id(&value)?,name:name(title)?},
            Self::DeleteBookmark{id:value}=>LocalViewChange::DeleteBookmark{id:id(&value)?},
        })
    }
}
#[uniffi::export]
impl ProjectSession {
    pub async fn local_views_snapshot(&self,binding:LocalViewsBinding,memory_budget_bytes:u64,
        cancellation:Arc<Cancellation>)->WorkflowResult<LocalViewsSnapshot> {
        self.check_open()?;
        let (project,document,device)=validate(&binding,memory_budget_bytes)?;
        let closed=self.closed.clone();
        self.worker.call(move|state|Ok((||{
            check_live(&closed,&cancellation)?;
            check_project(state,&project,&document)?;
            let value=state.store.as_mut().ok_or(WorkflowError::Closed)?.local_view(&device,&document).map_err(failure)?;
            check_live(&closed,&cancellation)?;
            Ok(snapshot(binding,value))
        })())).await?
    }
    pub async fn change_local_views(&self,binding:LocalViewsBinding,expected_revision:u64,operation_id:String,
        mutation:LocalViewsMutation,memory_budget_bytes:u64,cancellation:Arc<Cancellation>)->WorkflowResult<LocalViewsSnapshot> {
        self.check_open()?;
        let (project,document,device)=validate(&binding,memory_budget_bytes)?;
        let operation=id(&operation_id)?;let change=mutation.owned()?;
        let closed=self.closed.clone();
        self.worker.call(move|state|Ok((||{
            check_live(&closed,&cancellation)?;
            check_project(state,&project,&document)?;
            let value=state.store.as_mut().ok_or(WorkflowError::Closed)?.change_local_view(&device,&document,expected_revision,&operation,change,
                ||closed.load(Ordering::Acquire)||cancellation.is_cancelled()).map_err(failure)?;
            // Do not turn a successful durable local write into a reported
            // cancellation. The shared facade settles any undelivered result.
            Ok(snapshot(binding,value))
        })())).await?
    }
}
