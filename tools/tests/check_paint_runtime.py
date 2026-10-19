#!/usr/bin/env python3
"""Compile actual runtime inventory/catalog leaves and existing synthetic tests.

No DLL loading, generated bindings, Windows IO, GUI or app build is exercised.
"""
from pathlib import Path
import subprocess, os, tempfile
root=Path(__file__).resolve().parents[2]
tools=Path(os.environ.get('VW_TOOLS_ROOT','/workspace/.vw-tools'))
cache=tools/'gradle/caches/modules-2/files-2.1'
def jar(g,n,v):
 a=list((cache/g/n/v).glob('*/*.jar')); assert len(a)==1,(g,n,v);return a[0]
compiler=[jar('org.jetbrains.kotlin',n,'2.4.20') for n in ['kotlin-compiler-embeddable','kotlin-build-tools-api','kotlin-stdlib','kotlin-script-runtime','kotlin-daemon-embeddable']]
compiler += [jar('org.jetbrains.kotlin','kotlin-reflect','1.6.10'),jar('org.jetbrains.kotlinx','kotlinx-coroutines-core-jvm','1.8.0'),jar('org.jetbrains','annotations','13.0')]
cp=[jar('org.jetbrains.kotlin','kotlin-stdlib','2.4.20'),jar('net.java.dev.jna','jna','5.19.1'),jar('junit','junit','4.13.2'),jar('org.hamcrest','hamcrest-core','1.3')]
base=root/'apps/desktop/src'; names=['DesktopNativeRuntime','NativeRuntimeAnchor','DesktopRemoteCatalog','DesktopRemoteCatalogAdmission']
sources=[base/'main/kotlin/com/visualworkbench/desktop'/(n+'.kt') for n in names]
tests=['DesktopNativeRuntimeTest','NativeRuntimeAnchorTest','DesktopRemoteCatalogTest']
sources += [base/'test/kotlin/com/visualworkbench/desktop'/(n+'.kt') for n in tests]
java=Path(os.environ['JAVA_HOME'])/'bin/java'
with tempfile.TemporaryDirectory(prefix='vw-paint-runtime-') as d:
 out=Path(d)/'classes'
 subprocess.run([str(java),'-Xmx384m','-cp',os.pathsep.join(map(str,compiler)),'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler','-no-stdlib','-no-reflect','-jvm-target','21','-Werror','-classpath',os.pathsep.join(map(str,cp)),'-d',str(out),*map(str,sources)],check=True,timeout=55)
 subprocess.run([str(java),'-Xmx128m','-cp',os.pathsep.join(map(str,[out,*cp])),'org.junit.runner.JUnitCore',*['com.visualworkbench.desktop.'+n for n in tests]],check=True,timeout=25)
print('Actual native runtime/catalog leaves and existing tests passed; compiler outputs disposed. No ABI or Windows IO execution.')
