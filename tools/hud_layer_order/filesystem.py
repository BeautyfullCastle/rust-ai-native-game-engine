"""Filesystem/identity contracts; no process launch, Cargo, or source mutations."""
import hashlib,json,os,pathlib,re,stat,tomllib,zlib
MIB=1024**2

def require(value,message):
    if not value:raise RuntimeError(message)
def digest(path):
    h=hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda:stream.read(MIB),b''):h.update(chunk)
    return h.hexdigest()
def disk_inventory(root,exclude_git=False,hash_content=True):
    initial=root.lstat();require(stat.S_ISDIR(initial.st_mode),'inventory root must be ordinary directory')
    files={};allocated=initial.st_blocks*512;seen={(initial.st_dev,initial.st_ino)}
    for parent,dirs,names in os.walk(root,followlinks=False):
        parent=pathlib.Path(parent)
        if exclude_git and parent==root:dirs[:]=[name for name in dirs if name!='.git']
        for name in dirs+names:
            p=parent/name;s=p.lstat();require(not stat.S_ISLNK(s.st_mode),'symlink in admitted tree: '+str(p))
            if (s.st_dev,s.st_ino) not in seen:seen.add((s.st_dev,s.st_ino));allocated+=s.st_blocks*512
            if stat.S_ISDIR(s.st_mode):continue
            require(stat.S_ISREG(s.st_mode),'nonregular admitted file: '+str(p))
            if exclude_git:require(s.st_nlink==1,'source hardlink could mutate another checkout: '+str(p))
            info={'bytes':s.st_size,'mode':stat.S_IMODE(s.st_mode),'allocated':s.st_blocks*512,'device':str(s.st_dev),'inode':str(s.st_ino),'mtime_ns':str(s.st_mtime_ns),'uid':s.st_uid,'nlink':s.st_nlink}
            if hash_content:info['sha256']=digest(p)
            files[str(p.relative_to(root))]=info
    return files,allocated

def sources(root,expected_names):
    files,_=disk_inventory(root,exclude_git=True)
    require(set(files)==set(expected_names),'on-disk ignored/untracked/missing source input')
    return {p:{k:files[p][k] for k in ['sha256','bytes','mode']} for p in sorted(files)}

def git_identity(root):
    directory=root/'.git';require(directory.is_dir() and not directory.is_symlink(),'owned ordinary Git metadata directory required')
    metadata,_=disk_inventory(directory)
    require(sum(x['bytes'] for x in metadata.values())<=32*MIB,'Git metadata is outside bounded activation scope')
    def text(path):
        require(path.is_file() and not path.is_symlink() and path.stat().st_size<=MIB,'invalid Git reference')
        return path.read_text().strip()
    head=text(directory/'HEAD')
    if head.startswith('ref: '):
        reference=head[5:];path=pathlib.Path(reference)
        require(reference.startswith('refs/') and not path.is_absolute() and '..' not in path.parts,'unsafe HEAD symbolic ref')
        target=directory/path
        if target.exists():head=text(target)
        else:
            packed=text(directory/'packed-refs');matches=[line.split()[0] for line in packed.splitlines() if not line.startswith(('#','^')) and line.split()[-1:]==[reference]]
            require(len(matches)==1,'missing/ambiguous packed HEAD ref');head=matches[0]
    require(re.fullmatch('[0-9a-f]{40}',head) is not None,'invalid HEAD identity')
    path=directory/'objects'/head[:2]/head[2:]
    require(path.is_file() and not path.is_symlink() and path.stat().st_size<=MIB,'new candidate HEAD must have bounded loose commit object')
    decoder=zlib.decompressobj();raw=decoder.decompress(path.read_bytes(),MIB+1)
    require(len(raw)<=MIB and decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail,'invalid bounded Git commit object')
    header,body=raw.split(b'\0',1);require(header==b'commit '+str(len(body)).encode(),'wrong Git object type/size')
    require(hashlib.sha1(raw).hexdigest()==head,'Git commit object hash differs')
    headers=body.split(b'\n\n',1)[0].decode().splitlines()
    trees=[line[5:] for line in headers if line.startswith('tree ')];parents=[line[7:] for line in headers if line.startswith('parent ')]
    require(len(trees)==1 and re.fullmatch('[0-9a-f]{40}',trees[0]),'invalid commit tree')
    return {'head':head,'tree':trees[0],'parents':parents,'metadata':metadata}

def config_set(work,cargo_home):
    paths=set()
    for parent in [work,*work.parents]:paths.update([parent/'.cargo/config',parent/'.cargo/config.toml'])
    paths.update([cargo_home/'config',cargo_home/'config.toml'])
    found={}
    for path in sorted(paths):
        if path.exists() or path.is_symlink():
            require(path.is_file() and not path.is_symlink(),'nonregular Cargo config: '+str(path));found[str(path)]=digest(path)
    return found

def evidence_bytes(root):return sum(p.stat().st_size for p in root.rglob('*') if p.is_file() and p.suffix!='.gz')

def package_identities(root,manifest_names):
    """Exact Cargo output ID spellings derived only from pinned manifest/lock bytes.

    Cargo permits path+file:///.../foo#1.2.3 when basename equals package name,
    as well as an explicit #name@version. Registry IDs must match Cargo.lock.
    No arbitrary URL basename inference, external path, alias or fallback class.
    """
    identities={};local_names=set();workspace=tomllib.loads((root/'Cargo.toml').read_text()).get('workspace',{}).get('package',{})
    def add(identity,value):
        require(identity not in identities or identities[identity]==value,'ambiguous Cargo package identity')
        identities[identity]=value
    for name in manifest_names:
        if not (name=='Cargo.toml' or name.endswith('/Cargo.toml')):continue
        manifest=root/name;value=tomllib.loads(manifest.read_text()).get('package')
        if value is None:continue
        package=value['name'];version=value['version'];version=workspace.get('version') if version=={'workspace':True} else version;require(isinstance(version,str) and re.fullmatch('[A-Za-z0-9_-]+',package),'unsupported pinned package manifest')
        directory=manifest.parent;require(directory.resolve()==directory,'package directory alias');uri='path+'+directory.as_uri();owner={'package':package,'version':version,'class':'local','manifest':name};local_names.add(package)
        add(uri+'#'+package+'@'+version,owner)
        if directory.name==package:add(uri+'#'+version,owner)
    for value in tomllib.loads((root/'Cargo.lock').read_text())['package']:
        if 'source' not in value:continue
        source=value['source'];require(source.startswith('registry+https://'),'unsupported locked nonregistry external source');package=value['name'];version=value['version'];version=workspace.get('version') if version=={'workspace':True} else version;require(re.fullmatch('[A-Za-z0-9_-]+',package),'invalid locked package name')
        add(source+'#'+package+'@'+version,{'package':package,'version':version,'class':'registry','lock_source':source,'checksum':value['checksum']})
    return identities

def canonical_bin_proofs(provenance,target,start_ns,end_ns):
    """Exact audited rustc unit plus declared Cargo binary and current hardlink.

    Adapted from reviewed material-reload extern-successor canonical-bin mapping;
    additionally requires one shared inode, exactly two links, and equal full hash.
    """
    result={}
    for entry in provenance['local_artifacts']:
        row=entry['artifact']
        if row['target']['kind']!=['bin'] or row['profile']['test'] or not row.get('executable'):continue
        require(row['target']['crate_types']==['bin'] and row['fresh'] is False,'canonical bin must be fresh local bin')
        calls=entry['verbose'];require(len(calls)==1,'canonical bin invocation not unique');call=calls[0];tokens=call['tokens']
        option=lambda key:[tokens[i+1] for i,t in enumerate(tokens[:-1]) if t==key]
        extras=[v.removeprefix('extra-filename=-') for v in option('-C') if v.startswith('extra-filename=-')]
        require(len(extras)==1 and re.fullmatch('[0-9a-f]{16}',extras[0]),'canonical bin unit suffix differs')
        require(option('--out-dir')==[str(target/'release/deps')],'canonical bin compiler out-dir differs')
        require(call['manifest']==row['manifest_path'] and call['source']==row['target']['src_path'] and call['crate']==row['target']['name'].replace('-','_') and call['test'] is False and call['features']==sorted(row['features']),'canonical bin source/target/features mismatch')
        canonical=target/'release'/row['target']['name'];compiled=target/'release/deps'/(call['crate']+'-'+extras[0])
        require(row['filenames']==[str(canonical)] and row['executable']==str(canonical),'canonical bin JSON output differs')
        for path in [canonical,compiled]:require(path.is_file() and not path.is_symlink() and path.resolve()==path,'canonical bin path alias')
        a=canonical.stat();b=compiled.stat();require((a.st_dev,a.st_ino)==(b.st_dev,b.st_ino) and a.st_nlink==b.st_nlink==2,'canonical bin is not exact two-link inode')
        require(a.st_size==b.st_size and a.st_mtime_ns==b.st_mtime_ns and start_ns<=a.st_mtime_ns<=end_ns,'canonical bin is not within exact producer window')
        canonical_sha=digest(canonical);require(digest(compiled)==canonical_sha,'canonical bin full hash differs')
        signature=lambda st:(st.st_dev,st.st_ino,st.st_nlink,st.st_size,st.st_mtime_ns,st.st_ctime_ns,st.st_mode,st.st_blocks)
        require(signature(canonical.stat())==signature(a) and signature(compiled.stat())==signature(b),'canonical bin changed during fullhash readback')
        key=row['package_id']+'|'+row['target']['name'];require(key not in result,'duplicate canonical bin proof')
        result[key]={'artifact':row,'compiler_relative':str(compiled.relative_to(target)),'canonical_relative':str(canonical.relative_to(target)),'depinfo_relative':str(compiled.relative_to(target))+'.d','fingerprint_relative':'release/.fingerprint/'+pathlib.Path(row['manifest_path']).parent.name+'-'+extras[0],'sha256':canonical_sha,'bytes':a.st_size,'device':str(a.st_dev),'inode':str(a.st_ino),'nlink':a.st_nlink,'mtime_ns':str(a.st_mtime_ns),'producer_start_ns':str(start_ns),'producer_end_ns':str(end_ns)}
    return result

# Exact native release-profile sentinels, not a prefix/suffix ignore. Cargo's
# Layout::new opens these through Filesystem::open_rw_exclusive_create and
# retains the empty files after unlocking. This is filesystem metadata only;
# it does not prove producer success, unit provenance, or ownership quiescence.
# https://doc.rust-lang.org/nightly/nightly-rustc/src/cargo/compiler/layout.rs.html
CARGO_RELEASE_LOCKS=frozenset(['release/.cargo-build-lock','release/.cargo-artifact-lock'])

def cargo_release_lock_owner(target,name,value):
    require(name in CARGO_RELEASE_LOCKS,'unrecognized Cargo release lock')
    path=target/name
    require(target.is_absolute() and path.resolve()==path,'Cargo lock path alias')
    for directory in [target,path.parent]:
        info=directory.lstat()
        require(stat.S_ISDIR(info.st_mode) and info.st_uid==os.geteuid(),
                'Cargo lock parent must be an owned ordinary directory')
    info=path.lstat()
    require(stat.S_ISREG(info.st_mode),'Cargo lock must be an ordinary file')
    require(info.st_uid==os.geteuid() and info.st_nlink==1,'Cargo lock owner/link count differs')
    require(info.st_size==0 and stat.S_IMODE(info.st_mode) in [0o600,0o640,0o644],
            'Cargo lock must be empty, owner-writable, nonexecutable metadata')
    expected={'bytes':info.st_size,'mode':stat.S_IMODE(info.st_mode),'allocated':info.st_blocks*512,
              'device':str(info.st_dev),'inode':str(info.st_ino),'mtime_ns':str(info.st_mtime_ns),
              'uid':info.st_uid,'nlink':info.st_nlink}
    require(all(value.get(k)==v for k,v in expected.items()),'Cargo lock inventory differs')
    empty_sha=hashlib.sha256(b'').hexdigest()
    require('sha256' not in value or value['sha256']==empty_sha,'Cargo lock content hash differs')
    return {'class':'cargo-metadata','package':None,'metadata':'empty-release-lock',
            'path':name,'uid':info.st_uid,'nlink':info.st_nlink,'sha256':empty_sha}

def attribute_output_changes(before,after,rows,target,package_identities,canonical_bins=None):
    """Reject new/changed retained files without an exact emitted-unit/build-script owner.

Fingerprints and dep-info are bounded metadata tied to the same emitted unit hash;
all other payloads require an explicit compiler filename or build-script out_dir.
"""
    exact={};fingerprints={};buildroots={};canonical_bins=canonical_bins or {}
    def relative(value):
        p=pathlib.Path(value).resolve();require(p.is_relative_to(target),'Cargo output escaped target');return str(p.relative_to(target))
    def owner(row):
        identity=row['package_id'];require(identity in package_identities,'Cargo package identity absent from pinned manifests/lock: '+identity)
        return {**package_identities[identity],'package_id':identity,'target':row.get('target',{}).get('name')}
    for row in rows:
        if row.get('reason')=='compiler-artifact':
            who=owner(row)
            for filename in row['filenames']:
                name=relative(filename);exact[name]=who
                p=pathlib.Path(name)
                if p.parent==pathlib.Path('release/deps'):
                    base=p.name
                    match=re.search(r'-([0-9a-f]{16})(?:\.|$)',base)
                    if match:
                        unit=match[1];fingerprints['release/.fingerprint/'+who['package']+'-'+unit]=who
                        depbase=base.split('.',1)[0]
                        if depbase.startswith('lib'):depbase=depbase[3:]
                        exact[str(p.with_name(depbase+'.d'))]=who
                if len(p.parts)>=4 and p.parts[:2]==('release','build'):
                    buildroots['/'.join(p.parts[:3])]=who
                    fingerprint=p.parts[2];fingerprints['release/.fingerprint/'+fingerprint]=who
            alias=canonical_bins.get(row['package_id']+'|'+row['target']['name']) if row['target'].get('kind')==['bin'] and not row['profile']['test'] else None
            if alias is not None:
                require(alias['artifact']==row,'canonical bin proof row differs')
                exact[alias['compiler_relative']]={**who,'metadata':'exact-compiler-canonical-bin-hardlink','canonical_bin':alias}
                exact[alias['depinfo_relative']]={**who,'metadata':'exact-bin-depinfo'}
                fingerprints[alias['fingerprint_relative']]={**who,'metadata':'exact-bin-fingerprint'}
            executable=row.get('executable')
            if executable:
                name=relative(executable);exact[name]=who
                if pathlib.Path(name).parent==pathlib.Path('release'):exact[name+'.d']=who
        elif row.get('reason')=='build-script-executed':
            who=owner(row);out=pathlib.Path(relative(row['out_dir']))
            require(len(out.parts)>=4 and out.parts[:2]==('release','build') and out.name=='out','unrecognized build-script output location')
            buildroots[str(out.parent)]=who;fingerprints['release/.fingerprint/'+out.parent.name]=who
    changed={}
    for name,value in after.items():
        lock_owner=cargo_release_lock_owner(target,name,value) if name in CARGO_RELEASE_LOCKS else None
        if before.get(name)==value:continue
        p=pathlib.Path(name);who=lock_owner if lock_owner is not None else exact.get(name)
        if who is None:
            for prefix,entry in fingerprints.items():
                if p.parent==pathlib.Path(prefix) and value['bytes']<=16*MIB:who={**entry,'metadata':'fingerprint'};break
        if who is None:
            for prefix,entry in buildroots.items():
                if p.is_relative_to(pathlib.Path(prefix)):
                    tail=p.relative_to(prefix)
                    if tail.parts[0]=='out' or str(tail) in ['invoked.timestamp','output','stderr','root-output','build-script-build','build_script_build','build_script_build.d'] or re.fullmatch(r'build_script_build-[0-9a-f]{16}(?:\.d)?',str(tail)):
                        who={**entry,'metadata':'build-script'};break
        if who is None and name in ['.rustc_info.json','release/.cargo-lock','debug/.cargo-lock','.orr-hud-layer-order.lock','CACHEDIR.TAG']:
            require(value['bytes']<=MIB,'oversized Cargo control metadata');who={'class':'cargo-metadata','package':None}
        require(who is not None,'unrecognized retained Cargo output change: '+name)
        changed[name]={'owner':who,'before':before.get(name),'after':value}
    removed=sorted(set(before)-set(after))
    # Cargo may replace obsolete metadata, but every removed path must also be
    # attributable to an emitted unit; never let external cleanup offset growth.
    for name in removed:
        p=pathlib.Path(name)
        known=name in exact or any(p.is_relative_to(pathlib.Path(prefix)) for prefix in [*fingerprints,*buildroots])
        require(known,'unrecognized target removal during producer: '+name)
    return {'changed':changed,'removed':removed,'classes':sorted({x['owner']['class'] for x in changed.values()})}


def require_candidate_identity(identity,head,tree,base):
    require(identity['head']==head and identity['tree']==tree and identity['parents']==[base],
            'HEAD/tree/sole parent differs')

def require_archive_identity(receipt,identity):
    require(receipt['raw_sha256']==identity['sha256'] and receipt['raw_size']==identity['bytes'],
            'archive is not selected full producer')

def observe_allocation(root,max_entries=500000):
    """Bounded live observation; vanished leaves are recorded, never trusted as admission."""
    total=0;seen=set();races=0;entries=0
    for parent,dirs,names in os.walk(root,followlinks=False):
        for path in [pathlib.Path(parent),*[pathlib.Path(parent)/name for name in names]]:
            entries+=1;require(entries<=max_entries,'live allocation observation entry cap')
            try:info=path.lstat()
            except FileNotFoundError:races+=1;continue
            if (info.st_dev,info.st_ino) not in seen:seen.add((info.st_dev,info.st_ino));total+=info.st_blocks*512
    return {'allocated_bytes':total,'vanished_entries':races,'observed_entries':entries,'authoritative':False}
