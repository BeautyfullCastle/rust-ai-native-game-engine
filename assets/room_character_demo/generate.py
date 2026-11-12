#!/usr/bin/env python3
"""Original CC0 Orrery key courier: rigid-weighted robot, three authored loop clips.
Run manually with Python 3. Runtime and creator never execute this source.
"""
import hashlib, json, math
from pathlib import Path
root = Path(__file__).resolve().parent
identity = [[1,0,0,0],[0,1,0,0],[0,0,1,0],[0,0,0,1]]
pivots = [(0,0,0),(-.35,.65,0),(.35,.65,0),(-.16,-.03,0),(.16,-.03,0),(0,.92,0)]
names = ['courier_root','left_shoulder','right_shoulder','left_hip','right_hip','head']
nodes=[]
for i,(name,pivot) in enumerate(zip(names,pivots)):
 nodes.append(dict(name=name,children=list(range(1,6)) if i==0 else [],rest=dict(translation=pivot,rotation=[0,0,0,1],scale=[1,1,1])))
ibm=[]
for pivot in pivots:
 m=[v[:] for v in identity];m[3]=[-v for v in pivot]+[1];ibm.append(m)
primitives=[]
# Each cube has independent face normals and UVs, with one rigid skin weight.
def box(name,joint,center,size,material):
 vertices=[];indices=[]
 # outward counterclockwise winding as seen from outside
 faces=[((1,0,0),[(1,-1,-1),(1,1,-1),(1,1,1),(1,-1,1)]),((-1,0,0),[(-1,-1,1),(-1,1,1),(-1,1,-1),(-1,-1,-1)]),((0,1,0),[(-1,1,-1),(-1,1,1),(1,1,1),(1,1,-1)]),((0,-1,0),[(-1,-1,1),(-1,-1,-1),(1,-1,-1),(1,-1,1)]),((0,0,1),[(1,-1,1),(1,1,1),(-1,1,1),(-1,-1,1)]),((0,0,-1),[(-1,-1,-1),(-1,1,-1),(1,1,-1),(1,-1,-1)])]
 for normal,corners in faces:
  base=len(vertices)
  for corner,uv in zip(corners,[(0,0),(0,1),(1,1),(1,0)]):
   vertices.append(dict(vertex=dict(position=[center[a]+corner[a]*size[a]/2 for a in range(3)],normal=normal,uv=uv),joints=[joint,0,0,0],weights=[1,0,0,0]))
  indices += [base,base+1,base+2,base,base+2,base+3]
 primitives.append(dict(id="orrery-key-courier-v1#node=0/mesh="+name+"/primitive=0",node=0,skin=0,vertices=vertices,indices=indices,material=material))
box('blue_torso',0,(0,.45,0),(.58,.72,.32),0)
box('head',5,(0,1.10,0),(.47,.39,.39),1)
box('visor',5,(0,1.12,.205),(.33,.10,.04),2)
box('left_arm',1,(-.43,.37,0),(.18,.62,.20),1)
box('right_arm',2,(.43,.37,0),(.18,.62,.20),1)
box('left_leg',3,(-.16,-.24,0),(.20,.48,.24),0)
box('right_leg',4,(.16,-.24,0),(.20,.48,.24),0)
box('left_boot',3,(-.16,-.47,.07),(.24,.16,.38),2)
box('right_boot',4,(.16,-.47,.07),(.24,.16,.38),2)
def rotation(axis,degrees):
 half=math.radians(degrees)/2;v=[0,0,0,math.cos(half)];v[axis]=math.sin(half);return v

def clip(name,duration,tracks):
 return dict(name=name,channels=[dict(node=node,interpolation='linear',times=[0,duration/2,duration],values=dict(path='rotation',values=[rotation(axis,a) for a in angles])) for node,axis,angles in tracks])
clips=[clip('searching',2,[(1,0,[-18,18,-18]),(2,0,[18,-18,18]),(5,1,[-18,18,-18])]),clip('carrying_key',1.5,[(1,0,[-70,-60,-70]),(2,0,[-70,-60,-70]),(5,2,[-5,5,-5])]),clip('escaped',1,[(1,2,[-150,-125,-150]),(2,2,[150,125,150]),(5,1,[-12,12,-12])])]
materials=[dict(base_color=color,image=0,linear_filter=False,wrap_s='clamp',wrap_t='clamp') for color in [[.12,.55,.9,1],[.96,.68,.12,1],[.06,.08,.12,1]]]
model=dict(format='orr_animated_model',version=1,asset_id='orrery-key-courier-v1',dependencies=[dict(uri='$source',sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())],materials=materials,images=[dict(width=1,height=1,rgba8=[255,255,255,255])],nodes=nodes,skins=[dict(name='courier_rig',joints=list(range(6)),inverse_bind_matrices=ibm)],primitives=primitives,clips=clips)
(root/'courier.orrmodel.json').write_text(json.dumps(model,separators=(',',':'))+'\n')
