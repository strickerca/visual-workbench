#!/usr/bin/env python3
"""Compile the exact Paint source-scope predicate at its state/helper boundary.

Uses real cached vw-remote types; substitutes only the hub/locked-helper boundary.
Not a Windows, native ownership, whole-FFI or generated-binding test.
"""
from pathlib import Path
import subprocess
import tempfile
import argparse

args=argparse.ArgumentParser(description=__doc__)
args.add_argument('--negative-control',action='store_true')
options=args.parse_args()
root=Path(__file__).resolve().parents[2]
native=(root/'core/crates/vw-ffi/src/session/remote_compatibility.rs').read_text()
scope=native[native.index('struct ObservationContext {'):native.index('#[cfg(windows)]\nimpl Owned')]
scope=scope.replace('#[cfg(windows)]\n','').replace('vw_remote_host::editor_probe_runner::PaintObservationHelper','ProbeHelper').replace('pub(super) ', '')
if options.negative_control:
    old='\n            || s.epoch != self.local_epoch'
    assert old in scope
    scope=scope.replace(old,'',1)
remote=(root/'core/crates/vw-ffi/src/session/remote_edit.rs').read_text()
current=remote[remote.index('    fn current(s: &State)'):remote.index('    fn reset_source(')]
prefix='''use std::sync::{Arc,Mutex};
use vw_remote::{Binding,Scope,Target,Rect,profile::{ImageIdentity,Authority,AuthorityRoutes,CanvasProof}};
#[derive(Debug)] enum SessionError {Closed,Worker}
type SessionResult<T>=std::result::Result<T,SessionError>;
struct ProbeHelper;
struct Runtime {observation:Option<Arc<ProbeHelper>>}
struct Selected {target:Target,scope:Scope}
struct State {epoch:u64,source_epoch:Option<u64>,grant_revision:u64,closed:bool,enabled:bool,
    sender:Option<()>,host_retiring:bool,peer_retiring:bool,binding:Option<Binding>,
    selected:Option<Selected>,authority:Option<Authority>,runtime:Option<Runtime>}
struct RemoteHub {state:Mutex<State>}
'''
tests='''
#[cfg(test)] mod tests {
use super::*;
fn fixture()->SessionResult<(RemoteHub,ObservationContext)> {
 let scope=Scope {connection_epoch:1,capture_session_id:"018bcfe5-6800-7000-8000-000000000001".into(),
  source_generation:1,target_token:"018bcfe5-6800-7000-8000-000000000002".into(),geometry_revision:1};
 let binding=Binding {scope:scope.clone(),input_session_id:"018bcfe5-6800-7000-8000-000000000003".into()};
 let rect=Rect {x:0,y:0,width:100,height:100};
 let target=Target {token:scope.target_token.clone(),window:1,process_id:2,thread_id:3,process_created:4,
  window_rect:rect,frame_rect:rect,client_rect:rect,dpi:96,integrity:8192};
 target.validate().map_err(|_|SessionError::Worker)?;
 let image=ImageIdentity {executable_name:"mspaint.exe".into(),executable_blake3:"a".repeat(64),executable_bytes:128,
  file_version:None,package_full_name:Some("Paint_1.2.3.4_x64__test".into()),package_version:Some("1.2.3.4".into())};
 let authority=Authority {identity:image.clone(),profile_digest:"b".repeat(64),verified_actions:vec![1],
  routes:AuthorityRoutes::Essential,selected_canvas:None,focus:Some(CanvasProof {runtime_id_hash:"c".repeat(64),
   process_id:2,sampled_qpc_100ns:1,class_name:"canvas".into(),control_type:50026,canvas_rect:rect,profile_digest:"b".repeat(64)})};
 authority.validate().map_err(|_|SessionError::Worker)?;
 let helper=Arc::new(ProbeHelper);
 let context=ObservationContext {local_epoch:1,source_epoch:Some(1),grant_revision:1,binding:binding.clone(),
  target:target.clone(),identity:image,profile_digest:"b".repeat(64),helper:helper.clone()};
 let hub=RemoteHub {state:Mutex::new(State {epoch:1,source_epoch:Some(1),grant_revision:1,closed:false,enabled:true,
  sender:Some(()),host_retiring:false,peer_retiring:false,binding:Some(binding),selected:Some(Selected {target,scope}),
  authority:Some(authority),runtime:Some(Runtime {observation:Some(helper)})})};
 Ok((hub,context))
}
#[test] fn admits_only_the_same_actual_context()->SessionResult<()> {
 let (hub,context)=fixture()?;context.current(&hub)
}
#[test] fn rejects_local_epoch_reactivation_with_unchanged_binding()->SessionResult<()> {
 let (hub,context)=fixture()?;assert_eq!(context.local_epoch,1);
 hub.state.lock().map_err(|_|SessionError::Worker)?.epoch=3;
 assert!(context.current(&hub).is_err());Ok(())
}
#[test] fn refuses_grant_scope_image_profile_helper_and_retirement_changes()->SessionResult<()> {
 for field in 0..14 {
  let (hub,context)=fixture()?;
  {let mut s=hub.state.lock().map_err(|_|SessionError::Worker)?;
   match field {
    0=>s.grant_revision+=1,1=>s.source_epoch=Some(2),2=>s.closed=true,3=>s.enabled=false,
    4=>s.sender=None,5=>s.host_retiring=true,6=>s.peer_retiring=true,
    7=>s.runtime=Some(Runtime {observation:Some(Arc::new(ProbeHelper))}),8=>s.runtime=None,
    9=>s.selected.as_mut().ok_or(SessionError::Worker)?.scope.geometry_revision+=1,
    10=>s.selected.as_mut().ok_or(SessionError::Worker)?.target.process_created+=1,
    11=>s.authority.as_mut().ok_or(SessionError::Worker)?.identity.executable_blake3="d".repeat(64),
    12=>s.authority.as_mut().ok_or(SessionError::Worker)?.profile_digest="e".repeat(64),
    _=>s.binding.as_mut().ok_or(SessionError::Worker)?.input_session_id="new-session".into(),
   }}
  assert!(context.current(&hub).is_err(),"changed field {field}");
 } Ok(())
}
}
'''
deps=root/'target/debug/deps'
remote=max(deps.glob('libvw_remote-*.rlib'),key=lambda p:p.stat().st_mtime)
with tempfile.TemporaryDirectory(prefix='vw-paint-source-scope-') as directory:
    temporary=Path(directory);path=temporary/'scope.rs';output=temporary/'scope'
    path.write_text(prefix+scope+'impl RemoteHub {\n'+current+'}\n'+tests)
    subprocess.run(['rustc','--edition=2024','--test','-D','warnings','-L','dependency='+str(deps),
        '--extern','vw_remote='+str(remote),str(path),'-o',str(output)],check=True,timeout=45)
    result=subprocess.run([str(output),'--nocapture'],timeout=15)
    if options.negative_control:
        assert result.returncode!=0,'Old missing-local-epoch predicate unexpectedly passed'
        print('Expected negative-control failure caught stale local-epoch admission.')
    else:
        result.check_returncode()
        print('Exact source-scope predicate passed at substituted state/helper boundary; no Windows/FFI execution.')
