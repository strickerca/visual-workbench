#!/usr/bin/env python3
from pathlib import Path
import argparse,hashlib,json,os,re,subprocess,tempfile

parser=argparse.ArgumentParser(description='Focused actual giant-import ownership checks; final ABI adapter execution remains separate.')
parser.add_argument('--tools-root', type=Path, required=True)
parser.add_argument('--receipt-dir', type=Path, required=True)
args=parser.parse_args()
root=Path(__file__).resolve().parents[2]
tools=args.tools_root.resolve()
cache=tools/'gradle/caches/modules-2/files-2.1'
output=args.receipt_dir.resolve()
output.mkdir(exist_ok=False)
def jar(group,artifact,version):
    paths=list((cache/group/artifact/version).glob('*/*.jar'))
    assert len(paths)==1,(group,artifact,version,len(paths))
    return paths[0]
compiler=[jar('org.jetbrains.kotlin',name,'2.4.20') for name in
          ['kotlin-compiler-embeddable','kotlin-build-tools-api','kotlin-stdlib','kotlin-script-runtime','kotlin-daemon-embeddable']]
compiler += [jar('org.jetbrains.kotlin','kotlin-reflect','1.6.10'),
             jar('org.jetbrains.kotlinx','kotlinx-coroutines-core-jvm','1.8.0'),
             jar('org.jetbrains','annotations','13.0')]
stdlib=jar('org.jetbrains.kotlin','kotlin-stdlib','2.4.20')
coroutines=jar('org.jetbrains.kotlinx','kotlinx-coroutines-core-jvm','1.9.0')
junit=jar('junit','junit','4.13.2'); hamcrest=jar('org.hamcrest','hamcrest-core','1.3')
jna=jar('net.java.dev.jna','jna','5.19.1')
shared=root/'apps/shared/build/libs/shared-desktop.jar'
bindings=root/'apps/bindings-core/build/libs/bindings-core.jar'
java=tools/'jdk-root/usr/lib/jvm/java-21-openjdk-amd64/bin/java'
common=root/'apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/GiantImportsApi.kt'
owner=root/'apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/GiantImportOwnership.kt'
tests=root/'apps/shared/src/jvmTest/kotlin/com/visualworkbench/shared/GiantImportOwnershipTest.kt'
native_callers=root/'apps/shared/src/jvmTest/kotlin/com/visualworkbench/shared/GiantImportNativeCallerTest.kt'
commands=[]
def run(name,command):
    p=subprocess.run([str(x) for x in command],cwd=root,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,timeout=60)
    (output/(name+'.log')).write_text(p.stdout)
    commands.append({'name':name,'exit_code':p.returncode})
    print(name,p.returncode,p.stdout[-2000:],flush=True)
    assert p.returncode==0,name
try:
    with tempfile.TemporaryDirectory(prefix='vw-public-giant-ownership-') as tmp:
        tmp=Path(tmp); api=tmp/'api.jar'; cases=tmp/'cases.jar'
        base=[stdlib,coroutines,shared,bindings,junit,hamcrest,jna]
        compiler_base=[java,'-Xmx512m','-cp',os.pathsep.join(map(str,compiler)),
            'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler','-no-stdlib','-no-reflect','-jvm-target','21','-Werror']
        run('common-explicit-api',compiler_base+['-Xexplicit-api=strict','-classpath',os.pathsep.join(map(str,base)),'-d',api,common])
        run('ownership-and-callers-compile',compiler_base+['-classpath',os.pathsep.join(map(str,[api,*base])),
            '-Xfriend-paths='+str(shared),'-d',cases,owner,tests,native_callers])
        run('ownership-junit', [java,'-Xmx256m','-Djna.library.path='+str(root/'target/debug'),'-cp',
            os.pathsep.join(map(str,[cases,api,*base])),'org.junit.runner.JUnitCore',
            'com.visualworkbench.shared.GiantImportOwnershipTest'])
    result=(output/'ownership-junit.log').read_text()
    count=int(re.search(r'OK \((\d+) tests?\)',result).group(1))
    receipt={'status':'focused ownership/API probe passed','compiler':'Kotlin2.4.20','heap_mib':512,
        'junit_cases':count,'commands':commands,'source_sha256':{str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest()
          for p in [common,owner,tests,native_callers]},
        'limits':['Actual shared ownership algorithm and existing joined native Cancellation bridge exercised',
                  'Three native facade callers compile only; execution deferred until final new ABI generation',
                  'Thin NativeGiantImports adapter and modified NativeCore not whole-module compiled here',
                  'Cached predecessor shared/bindings/native library used only for unchanged core types and cancellation symbols',
                  'No app, packaging, device or platform producer approval'],
        'temporary_compiler_outputs_disposed':True,
        'predecessor_compile_inputs':{str(p.relative_to(root)):hashlib.file_digest(p.open('rb'),'sha256').hexdigest() for p in [shared,bindings]}}
    (output/'receipt.json').write_text(json.dumps(receipt,indent=2)+'\n')
except Exception:
    (output/'failure.json').write_text(json.dumps({'commands':commands,'status':'failed; preserve logs'},indent=2)+'\n')
    raise
