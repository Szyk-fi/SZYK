import math,struct,hashlib
from pathlib import Path
count=4410
pcm=b''.join(struct.pack('<hh',int(8000*math.sin(math.tau*440*i/44100)),int(6000*math.sin(math.tau*660*i/44100))) for i in range(count))
samples=list(struct.iter_unpack('<hh',pcm))
def crc(data,bits,poly):
 c=0
 for b in data:
  c^=b<<(bits-8)
  for _ in range(8):c=((c<<1)^poly if c&(1<<(bits-1)) else c<<1)&((1<<bits)-1)
 return c
info=struct.pack('>HH',256,256)+b'\0'*6+((44100<<44)|(1<<41)|(15<<36)|count).to_bytes(8,'big')+hashlib.md5(pcm).digest()
result=b'fLaC'+bytes([0x80])+len(info).to_bytes(3,'big')+info
for f,start in enumerate(range(0,count,256)):
 chunk=samples[start:start+256];header=bytes([0xff,0xf8,0x69,0x18,f,len(chunk)-1]);frame=header+bytes([crc(header,8,0x07)])
 for channel in range(2):frame+=b'\x02'+b''.join(struct.pack('>h',x[channel]) for x in chunk)
 result+=frame+crc(frame,16,0x8005).to_bytes(2,'big')
Path(__file__).with_name('stereo.flac').write_bytes(result)
