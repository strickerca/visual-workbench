"""Record/check native or APK bytes against their source and gated build inputs."""
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]

def bindings(kind):
    names = ['build.ps1', 'tools/bench/video-pc/build_receipt.py']
    if kind == 'pc':
        names += ['Cargo.toml','Cargo.lock','rust-toolchain.toml','.cargo/config.toml','deny.toml',
                  'tools/bench/video-pc/Cargo.toml','target/release/video-pc.exe']
        paths = sorted((ROOT/'tools/bench/video-pc/src').rglob('*.rs'))
    elif kind == 'android':
        names += ['apps/build.gradle.kts','apps/settings.gradle.kts','apps/gradle/libs.versions.toml',
                  'apps/gradle.properties','tools/check_gradle_licenses.py',
                  'tools/bench/video-android/build.gradle.kts','tools/bench/video-android/gradle.lockfile',
                  'tools/bench/video-android/build/outputs/apk/debug/video-bench-debug.apk']
        paths = sorted((ROOT/'tools/bench/video-android/src').rglob('*'))
        paths = [p for p in paths if p.is_file()]
    else:
        raise ValueError('Unknown video build')
    paths += [ROOT/n for n in names]
    return {p.relative_to(ROOT).as_posix():hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}

def main():
    mode,kind=sys.argv[1:]
    current=bindings(kind)
    path=ROOT/'.local'/f'video-build-{kind}.json'
    if mode=='record':
        path.parent.mkdir(exist_ok=True)
        path.write_text(json.dumps({'schema':1,'sha256':current},indent=2)+'\n',encoding='utf-8',newline='\n')
    elif mode=='check':
        if json.loads(path.read_text())!={'schema':1,'sha256':current}:
            raise ValueError('Build source differs')
    else: raise ValueError('Unknown operation')
    print('Video '+kind+' build binding '+mode+': PASS')

if __name__=='__main__':
    try: main()
    except (OSError,ValueError,KeyError):
        raise SystemExit('Video build receipt missing, invalid or stale; gated rebuild required') from None
