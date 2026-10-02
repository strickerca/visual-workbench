"""Dispose exact task-owned verification media; retain only hashes/counts and JSON."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import tempfile

ROOT=Path(__file__).resolve().parents[3]

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run_id')
    parser.add_argument('--inspected',action='store_true',required=True)
    args=parser.parse_args()
    if not re.fullmatch('[0-9a-f]{32}',args.run_id):raise ValueError('Invalid run ID')
    record=ROOT/'.local'/f'video-run-{args.run_id}.json'
    meta=json.loads(record.read_text(encoding='utf-8-sig'))
    directory=Path(meta['media_directory']).resolve()
    expected=(Path(tempfile.gettempdir())/f'VW-video-{args.run_id}').resolve()
    if directory!=expected or meta['run_id']!=args.run_id:raise ValueError('Media ownership mismatch')
    receipt=ROOT/'.local'/f'video-disposal-{args.run_id}.json'
    if meta.get('media_disposed'):
        if not receipt.exists():raise ValueError('Missing prior disposal receipt')
        print('Owned video media already disposed; prior receipt retained')
        return
    entries=[]
    if directory.exists():
        for path in directory.rglob('*'):
            if path.is_file() and path.suffix in ('.png','.h265','.vwt'):
                resolved=path.resolve()
                if not resolved.is_relative_to(directory) or path.is_symlink():raise ValueError('Unexpected media containment')
                entries.append({'path':path.relative_to(directory).as_posix(),'bytes':path.stat().st_size,
                                'sha256':hashlib.sha256(path.read_bytes()).hexdigest()})
        for item in entries:
            path=directory/item['path']
            if not path.resolve().is_relative_to(directory):raise ValueError('Cleanup containment changed')
            path.unlink()
        if any(p.is_file() and p.suffix in ('.png','.h265','.vwt') for p in directory.rglob('*')):
            raise ValueError('Media disposal incomplete')
    receipt.write_text(json.dumps({'schema':1,'run_id':args.run_id,'inspected':True,'media':entries,
                                  'disposed_count':len(entries),'retained_media_count':0},indent=2)+'\n',encoding='utf-8')
    meta['media_disposed']=True
    record.write_text(json.dumps(meta,indent=2)+'\n',encoding='utf-8')
    print('Disposed',len(entries),'owned verification media files; only JSON/text and hashes remain')

if __name__=='__main__':
    try:main()
    except (OSError,ValueError,KeyError):raise SystemExit('Media ownership/disposal validation failed') from None
