"""Bounded loopback server for generated tile delivery/decode/posting over adb TCP."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import socket
import struct
import time

def read_exact(stream, count):
    data=bytearray()
    while len(data)<count:
        block=stream.recv(count-len(data))
        if not block: raise ValueError('Incomplete frame acknowledgment')
        data.extend(block)
    return bytes(data)

def stats(values):
    ordered=sorted(values)
    return {'count':len(values),'p50':ordered[math.ceil(.5*len(values))-1],
            'p95':ordered[math.ceil(.95*len(values))-1],'max':ordered[-1]}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input',type=Path)
    parser.add_argument('output',type=Path)
    parser.add_argument('--profile',choices=['smoke','full'],default='full')
    args=parser.parse_args()
    if not 20<args.input.stat().st_size<=128*1024*1024: raise ValueError('Invalid recording size')
    data=args.input.read_bytes()
    if data[:8]!=b'VWJT0002': raise ValueError('Invalid recording header')
    width,height,count=struct.unpack_from('>III',data,8)
    if (width,height)!=(2560,1440) or not 1<=count<=60: raise ValueError('Invalid recording dimensions/count')
    frames=[]; offset=20
    for _ in range(count+1):
        length=struct.unpack_from('>I',data,offset)[0]; offset+=4
        if not 4<=length<=8*1024*1024 or offset+length>len(data): raise ValueError('Invalid frame size')
        frames.append(data[offset:offset+length]); offset+=length
    if offset!=len(data): raise ValueError('Trailing recording data')
    baseline,frames=frames[0],frames[1:]
    args.output.mkdir()
    measured,warmup=(120,10) if args.profile=='full' else (10,2)
    samples=[]
    with socket.socket(socket.AF_INET,socket.SOCK_STREAM) as server:
        server.bind(('127.0.0.1',0));server.listen(1);server.settimeout(45)
        (args.output/'ready.json').write_text(json.dumps({'port':server.getsockname()[1]}))
        connection,_=server.accept()
        with connection:
            connection.settimeout(10);connection.setsockopt(socket.IPPROTO_TCP,socket.TCP_NODELAY,1)
            connection.sendall(b'VWJS0001'+struct.pack('>IIII',width,height,measured+warmup+1,warmup+1))
            measured_start=None;measured_bytes=0
            for i in range(measured+warmup+1):
                frame=baseline if i==0 else frames[(i-1)%len(frames)]
                if i==warmup+1: measured_start=time.perf_counter()
                start=time.perf_counter()
                connection.sendall(struct.pack('>II',i,len(frame))+frame)
                ack=read_exact(connection,36)
                elapsed=(time.perf_counter()-start)*1000
                if ack[:4]!=struct.pack('>I',i) or ack[4:]!=hashlib.sha256(frame).digest(): raise ValueError('Frame acknowledgment mismatch')
                if i>=warmup+1: samples.append(elapsed);measured_bytes+=len(frame)
                if i%15==0: print(f'USB tile delivery: {i+1}/{measured+warmup+1} acknowledged',flush=True)
            seconds=time.perf_counter()-measured_start
    result={'schema':1,'completed':True,'profile':args.profile,'recording_sha256':hashlib.sha256(data).hexdigest(),
            'recorded_frames':len(frames),'measured_frames':measured,'warmup_frames':warmup,'initial_frame_excluded':1,
            'measured_payload_bytes':measured_bytes,'elapsed_seconds':seconds,'frames_per_second':measured/seconds,
            'frame_send_to_decode_post_ack_ms':stats(samples),'raw_frame_ms':samples,
            'scope':'Physical adb TCP plus phone JPEG decode/reconstruction/Surface posting; recorded generated frames cycled, not concurrent live PC capture/encode or photon latency'}
    (args.output/'result.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8',newline='\n')
    print(f'USB JPEG tile delivery/phone decode/posting: {result["frames_per_second"]:.2f} frames/s',flush=True)

if __name__=='__main__':
    try:main()
    except (OSError,ValueError,struct.error):raise SystemExit('Tile transfer failed; inspect phase receipts') from None
