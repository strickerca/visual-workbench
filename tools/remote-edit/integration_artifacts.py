"""Hash-only input inventory. This never builds, launches, installs or claims a pass."""
import argparse,hashlib,json,os,re,stat
from pathlib import Path
import android_entrypoint_preflight

NATIVES=('vw_core.dll','vw_host.dll','vw-connection-helper.exe','vw-capture-helper.exe','vw-hevc-helper.exe','vw-input-helper.exe')
EXTRA=(
    'tools/remote-edit/input_observation_evidence.ps1','tools/remote-edit/test_input_observation_evidence.ps1',
    'tools/remote-edit/test_integration_diagnostics.ps1',
    'tools/remote-edit/receiver_observation.ps1','tools/remote-edit/test_receiver_observation.ps1',
    'apps/desktop/src/test/kotlin/com/visualworkbench/desktop/RemoteIntegrationPublicationTest.kt','tools/remote-edit/test_integration_producer_wait.ps1','tools/remote-edit/test_startup_owner_proofs.ps1','tools/ffi-test/core-unit.ps1','tools/ffi-test/test_core_unit_census.ps1',
    'tools/remote-edit/android_entrypoint_preflight.py','tools/remote-edit/test_android_entrypoint_preflight.py','tools/remote-edit/remote_instrumentation_contract.json',
    'tools/remote-edit/render_observation_evidence.ps1','tools/remote-edit/test_render_observation_evidence.ps1',
    'apps/android/build.gradle.kts',
    'apps/android/src/remoteIntegration/AndroidManifest.xml',
    'apps/android/src/remoteIntegration/kotlin/com/visualworkbench/android/remote/RemoteIntegrationActivity.kt',
    'apps/android/src/remoteIntegrationTest/kotlin/com/visualworkbench/android/remote/RemoteNormalPathInstrumentedTest.kt',
    'apps/desktop/src/main/kotlin/com/visualworkbench/desktop/RemoteIntegrationHost.kt',
    'apps/desktop/src/main/kotlin/com/visualworkbench/desktop/RemoteIntegrationReadiness.kt',
    'apps/desktop/src/test/kotlin/com/visualworkbench/desktop/RemoteIntegrationReadinessTest.kt',
    'tools/remote-edit/integration.ps1','tools/remote-edit/integration_artifacts.py',
    'tools/remote-edit/integration_reports.py','tools/remote-edit/test_integration_reports.py',
    'tools/remote-edit/controller_lifecycle_reports.py','tools/remote-edit/test_controller_lifecycle_reports.py',
    'tools/remote-edit/lifecycle_clock.cs',
    'core/crates/vw-ffi/Cargo.toml',
    'core/crates/vw-ffi/src/session/remote_integration_fault.rs',
    'apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/RemoteIntegrationCarrierFault.kt',
    'tools/remote-edit/input_owner_contracts.ps1','tools/remote-edit/input_owner_startup.ps1',
    'tools/remote-edit/input_process_output.cs','tools/remote-edit/test_input_owner_startup.ps1','tools/remote-edit/integration_input_owner.ps1',
    'tools/remote-edit/test_input_owner_contracts.ps1','tools/remote-edit/test_retained_input_wait.ps1',
    'tools/remote-edit/test_stream_counter_reports.ps1',
    'apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/RemoteGhostInk.kt',
    'apps/shared/src/commonTest/kotlin/com/visualworkbench/shared/RemoteGhostInkTest.kt',
    'tools/remote-edit/controller_input_reports.py','tools/remote-edit/test_controller_input_reports.py',
    'host-win/crates/vw-remote-host/src/hil.rs',
    'host-win/crates/vw-remote-host/src/hil/controller_surface.rs')
REQUIRED=(
    'host-win/crates/vw-remote-host/src/platform/shortcuts/krita_controls/wheel.rs',
    'host-win/crates/vw-remote-host/src/editor_effect/krita/wheel.rs',
    'apps/android/src/main/kotlin/com/visualworkbench/android/remote/RemoteEditController.kt',
    'apps/android/src/main/kotlin/com/visualworkbench/android/remote/RemoteDecoderRecovery.kt',
    'apps/android/src/test/kotlin/com/visualworkbench/android/remote/RemoteDecoderRecoveryTest.kt',
    'apps/android/src/main/kotlin/com/visualworkbench/android/remote/RemoteHardwareDecoder.kt',
    'apps/android/src/main/kotlin/com/visualworkbench/android/remote/RemoteSurfaceView.kt',
    'apps/desktop/src/main/kotlin/com/visualworkbench/desktop/DesktopNativeRuntime.kt',
    'apps/desktop/build.gradle.kts',
    'apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeRemoteEdit.kt',
    'core/crates/vw-ffi/src/session/remote_edit.rs',
    'core/crates/vw-ffi/src/session/remote_host.rs',
    'host-win/crates/vw-remote-host/src/process.rs',
    'host-win/crates/vw-remote-host/src/process/windows.rs',
    'host-win/crates/vw-remote-host/src/process/input_retirement.rs',
    'host-win/crates/vw-remote-host/src/platform/native_guard/release_witness.rs',
    'host-win/crates/vw-remote-host/src/platform/input.rs',
    'host-win/crates/vw-remote-host/src/platform/video.rs')

def require(ok,reason):
    if not ok:raise ValueError(reason)
def plain(path):
    p=Path(os.path.abspath(path))
    for entry in (p,*p.parents):
        info=entry.lstat()
        require(not stat.S_ISLNK(info.st_mode) and not (getattr(info,'st_file_attributes',0)&0x400),'redirected_path')
    return p
def digest(path):
    p=plain(path);require(p.is_file() and 0<p.stat().st_size<=2*1024**3,'file_bound')
    h=hashlib.sha256()
    with p.open('rb') as stream:
        for chunk in iter(lambda:stream.read(1024*1024),b''):h.update(chunk)
    return h.hexdigest()
def binding(path):
    p=plain(path);return {'path':str(p),'sha256':digest(p)}
def relative(value):
    require(isinstance(value,str) and re.fullmatch(r'[A-Za-z0-9_./-]+',value) and not value.startswith('/') and '..' not in value.split('/'),'source_path')
    return value
def main():
    a=argparse.ArgumentParser(description=__doc__)
    for name in ('root','source-map','java','classpath','apk-main','apk-test','harness','android-core','output'):
        a.add_argument('--'+name,required=True)
    a.add_argument('--receipt',action='append',required=True,help='Fresh root build/check receipt; reviewed by root, never inferred as pass here')
    a.add_argument('--android-integration-carrier-fault',action='store_true',help='Root-reviewed Android fixture build used feature integration-carrier-fault; never ordinary packaging')
    a.add_argument('--host-integration-carrier-fault',action='store_true',help='Root-reviewed Windows fixture core used feature integration-carrier-fault; never ordinary packaging')
    args=a.parse_args();root=plain(args.root);require(root.is_dir(),'root')
    source_map=plain(args.source_map);require(source_map.stat().st_size<=1024**2,'source_map_bound')
    sources=json.loads(source_map.read_text(encoding='utf-8-sig'));require(isinstance(sources,dict) and 25<=len(sources)<=2048,'source_census')
    require(set(REQUIRED)<=set(sources),'critical_pipeline_sources')
    rows=[]
    for name,expected in sorted(sources.items()):
        name=relative(name);require(re.fullmatch(r'[a-f0-9]{64}',expected),'source_digest')
        actual=digest(root/name);require(actual==expected,'current_source_changed')
        rows.append({'path':name,'sha256':actual})
    for name in EXTRA:
        if name not in sources:rows.append({'path':name,'sha256':digest(root/name)})
    cp_file=plain(args.classpath);require(cp_file.stat().st_size<=65536,'classpath_bound')
    cp=cp_file.read_text(encoding='utf-8-sig').strip();require(cp and len(cp)<=65536,'classpath')
    files={}
    for entry in cp.split(os.pathsep):
        p=plain(entry)
        if p.is_file():files[str(p)]=binding(p)
        else:
            require(p.is_dir(),'classpath_entry')
            for base,dirs,names in os.walk(p,followlinks=False):
                for item in dirs:plain(Path(base)/item)
                for item in names:
                    q=plain(Path(base)/item);files[str(q)]=binding(q)
                    require(len(files)<=32768,'classpath_census')
    require(10<=len(files)<=32768,'classpath_census')
    require(len(args.receipt)>=3,'fresh_build_receipts_missing')
    receipts=[binding(p) for p in args.receipt]
    entrypoint=android_entrypoint_preflight.verify(args.apk_test,root/'tools/remote-edit/remote_instrumentation_contract.json','com.visualworkbench.android.remote.RemoteNormalPathInstrumentedTest')
    out={'schema':1,'inventory_only':True,'root_build_review_required':True,
         'entrypoint_preflight':entrypoint,
         'host_fixture_features':['integration-carrier-fault'] if args.host_integration_carrier_fault else [],
         'android_fixture_features':['integration-carrier-fault'] if args.android_integration_carrier_fault else [],
         'source_map':binding(source_map),'sources':sorted(rows,key=lambda x:x['path']),
         'classpath_bindings':sorted(files.values(),key=lambda x:x['path']),
         'java':binding(args.java),'classpath':binding(cp_file),
         'native':{name:digest(root/'target/debug'/name) for name in NATIVES},
         'apk':{'main':binding(args.apk_main),'test':binding(args.apk_test)},
         'harness':binding(args.harness),'android_core_sha256':digest(args.android_core),
         'build_receipts':receipts,'executed':False}
    require(out['apk']['test']['sha256']==entrypoint['apk_sha256'],'entrypoint_apk_changed')
    target=Path(args.output).absolute();require(not target.exists(),'output_exists');plain(target.parent)
    with target.open('x',encoding='utf-8',newline='\n') as stream:json.dump(out,stream,indent=2);stream.write('\n')
    print(json.dumps({'artifact_manifest_sha256':digest(target),'sources':len(rows),'classpath_files':len(files),'executed':False}))
if __name__=='__main__':main()
