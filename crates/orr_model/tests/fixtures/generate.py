#!/usr/bin/env python3
"""Original Orrery fixture, MIT OR Apache-2.0. Standard library only.
Rebuilds genuine glTF 2.0 external and GLB embedded twins, not cooked data.
An asymmetric five-sided panel and separate marker have two material slots,
interleaved POS/NORMAL/UV0, indexed triangles and a transformed child node.
"""
import json, math, struct, zlib
from pathlib import Path
root=Path(__file__).parent
def png(pixels):
    def chunk(kind,data):
        return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)
    raw=b''.join(b'\0'+bytes(pixels[i:i+8]) for i in (0,8))
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',2,2,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b'')
quadrants=png([255,0,0,255,0,255,0,255,0,0,255,255,128,128,128,255])
marker=png([255,255,0,255]*4)
(root/'quadrants.png').write_bytes(quadrants);(root/'marker.png').write_bytes(marker)
panel=[(-1,-1,0),(1,-1,0),(1,.4,0),(.4,1,0),(-1,1,0)]
tri=[(1.1,-.75,0),(1.8,-.75,0),(1.4,.4,0)]
data=bytearray()
for p in panel+tri:
    uv=((p[0]+1)/2,(1-p[1])/2) if p in panel else (.25,.25)
    data+=struct.pack('<8f',*p,0,0,1,*uv)
data+=struct.pack('<9H',0,1,2,0,2,3,0,3,4)
while len(data)%4:data+=b'\0'
idx1=len(data);data+=struct.pack('<3H',0,1,2)
while len(data)%4:data+=b'\0'
views=[{'buffer':0,'byteOffset':0,'byteLength':256,'byteStride':32,'target':34962},
       {'buffer':0,'byteOffset':256,'byteLength':18,'target':34963},
       {'buffer':0,'byteOffset':idx1,'byteLength':6,'target':34963}]
accessors=[]
for start,count,points in [(0,5,panel),(160,3,tri)]:
    for kind,off in [('VEC3',0),('VEC3',12),('VEC2',24)]:
        a={'bufferView':0,'byteOffset':start+off,'componentType':5126,'count':count,'type':kind}
        if off==0:
            # Round bounds to the actual stored IEEE f32, not source Python doubles.
            f=lambda v:struct.unpack('<f',struct.pack('<f',v))[0]
            a.update(min=[f(min(p[i] for p in points)) for i in range(3)],max=[f(max(p[i] for p in points)) for i in range(3)])
        accessors.append(a)
accessors += [{'bufferView':1,'componentType':5123,'count':9,'type':'SCALAR'},
              {'bufferView':2,'componentType':5123,'count':3,'type':'SCALAR'}]
angle=math.radians(15)/2
doc={'asset':{'version':'2.0','generator':'Original Orrery fixture generator','copyright':'MIT OR Apache-2.0'},
     'scene':0,'scenes':[{'nodes':[0]}],
     'nodes':[{'name':'root offset','translation':[-.4,-.1,0],'children':[1]},
              {'name':'rotated nonuniform child','mesh':0,'translation':[.3,.1,.1],'rotation':[0,0,math.sin(angle),math.cos(angle)],'scale':[.8,1.1,1]}],
     'meshes':[{'name':'notched panel and marker','primitives':[
         {'attributes':{'POSITION':0,'NORMAL':1,'TEXCOORD_0':2},'indices':6,'material':0},
         {'attributes':{'POSITION':3,'NORMAL':4,'TEXCOORD_0':5},'indices':7,'material':1}]}],
     'buffers':[{'uri':'model.bin','byteLength':len(data)}],'bufferViews':views,'accessors':accessors,
     'images':[{'uri':'quadrants.png','mimeType':'image/png'},{'uri':'marker.png','mimeType':'image/png'}],
     'samplers':[{'magFilter':9728,'minFilter':9728,'wrapS':33071,'wrapT':33071}],
     'textures':[{'source':0,'sampler':0},{'source':1,'sampler':0}],
     'materials':[{'pbrMetallicRoughness':{'baseColorTexture':{'index':0},'metallicFactor':0,'roughnessFactor':1}},
                  {'pbrMetallicRoughness':{'baseColorTexture':{'index':1},'baseColorFactor':[.5,1,1,1],'metallicFactor':0,'roughnessFactor':1}}]}
(root/'model.bin').write_bytes(data)
(root/'model.gltf').write_text(json.dumps(doc,indent=2)+'\n')
for i,image in enumerate([quadrants,marker]):
    offset=len(data);data+=image
    views.append({'buffer':0,'byteOffset':offset,'byteLength':len(image)})
    doc['images'][i]={'bufferView':len(views)-1,'mimeType':'image/png'}
    while len(data)%4:data+=b'\0'
doc['buffers']=[{'byteLength':len(data)}]
jsonbytes=json.dumps(doc,separators=(',',':')).encode()
while len(jsonbytes)%4:jsonbytes+=b' '
glb=struct.pack('<III',0x46546c67,2,12+8+len(jsonbytes)+8+len(data))
glb+=struct.pack('<II',len(jsonbytes),0x4e4f534a)+jsonbytes
glb+=struct.pack('<II',len(data),0x004e4942)+data
(root/'model.glb').write_bytes(glb)
