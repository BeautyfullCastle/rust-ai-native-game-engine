"""Exact combined feature, inventory, fixture and copy contracts."""
import hashlib,json,pathlib,re,stat,struct,zlib
MIB=1024**2
GIB=1024**3
EDITOR_FEATURES=['collect-dodge','collect-ui','project-create','project-ui','sprites']
TOOL_FEATURES=['collect-dodge','collect-progress','collect-sprites','collect-ui','game-ui','input-actions','project','project-create','project-export','sprites']
CPU_TESTS=['editor_play_layer_order_readonly','layer_order_clipping_and_container_occlusion','layer_order_keyboard_focus_and_pointer_ownership']
GPU_MAIN='gpu_saved_layer_order_source_hidden_readonly_export_preserves_frames'
GPU_HELPER='layer_order_relocated_bundle_probe'
GPU_TESTS=[GPU_MAIN,GPU_HELPER]
TOOLS=['orr_new_arena','collect_dodge','orr_export_collect']
INFRA=['TIMEOUT','BWRAP','SHA256SUM','SHELL','LS','DIRNAME']
CAPTURES=['01-original-t0','02-back-t0','03-front-t0','04-undo-t0','05-redo-t0']+[f'08-probe-{i}-{where}' for i in range(4) for where in ['source','exported']]
COMMANDS=['00-generate',*CAPTURES[:5],'06-export','07-relocated-proof',*CAPTURES[5:]]

def require(value,message):
    if not value:raise RuntimeError(message)

def digest(path):
    h=hashlib.sha256()
    with path.open('rb') as stream:
        for data in iter(lambda:stream.read(MIB),b''):h.update(data)
    return h.hexdigest()

def canonical(value):
    p=pathlib.Path(value)
    require(p.is_absolute() and '..' not in p.parts,'noncanonical absolute path')
    return p

def object_digest(value):
    return hashlib.sha256(json.dumps(value,sort_keys=True,separators=(',',':')).encode()).hexdigest()

def source_matches_tree(work, listing, baseline):
    """Bind every input byte/mode to the requested Git tree, without stat-cache trust."""
    require(listing.endswith('\0'), 'unterminated Git tree listing')
    seen=set()
    for row in listing[:-1].split('\0'):
        match=re.fullmatch(r'(100644|100755) blob ([0-9a-f]{40})\t([^\0]+)',row)
        require(match is not None,'nonregular or malformed Git tree leaf')
        mode,expected,name=match.groups();path=pathlib.Path(name)
        require(not path.is_absolute() and '..' not in path.parts and str(path)==name,'Git tree path alias')
        require(name not in seen and name in baseline,'duplicate/unexpected Git tree leaf');seen.add(name)
        item=baseline[name];require(item['mode']==int(mode,8)&0o777,'Git tree executable mode differs')
        actual=hashlib.sha1(('blob '+str(item['bytes'])+'\0').encode())
        with (work/path).open('rb') as stream:
            for chunk in iter(lambda:stream.read(MIB),b''):actual.update(chunk)
        require(actual.hexdigest()==expected,'tracked bytes differ from requested Git tree: '+name)
    require(seen==set(baseline),'Git tree/source closure differs')
    return {'leaves':len(seen),'all_blob_hashes_and_modes_match':True}

def command(action,package,target=None):
    require(package in ['orr_editor','orr_sample'],'unapproved root package')
    args=['cargo',action,'--release','--locked','--offline','-j1','-p',package,'--no-default-features','--features',
          'collect-ui,project-create' if package=='orr_editor' else 'project-create,project-export,collect-ui,collect-sprites,collect-progress']
    if action=='test':
        require(package=='orr_editor' and target in ['ui_layer_order_acceptance','ui_layer_order_gpu_export'],'wrong integration target')
        args+=['--test',target,'--no-run']
    elif action=='build':
        require(package=='orr_sample' and target is None,'wrong tool producer')
        for tool in TOOLS:args+=['--bin',tool]
    elif action=='clippy':
        require(package=='orr_editor' and target is None,'wrong combined strict root')
        args+=['--all-targets']
    else:raise RuntimeError('unapproved producer action')
    args+=['--message-format=json','-vv']
    if action=='clippy':args+=['--','-D','warnings']
    return args

def selected_roots(rows,work,package,selections,features):
    """Generic package selection, never hard-coded to orr_editor/test=true.

    Call ONLY after unchanged audit_producer has validated all local fresh
    artifacts, exact verbose compiler/extern/source/profile/features identities.
    selections is {target_name: (kind, profile_test_bool)}.
    """
    expected_manifest=str(work/'crates'/package/'Cargo.toml')
    require(rows and all(row['manifest_path']==expected_manifest for row in rows),'selected root manifest mismatch')
    require(all(row['features']==sorted(features) or sorted(row['features'])==sorted(features) for row in rows),'root feature closure mismatch')
    selected={};auxiliary_seen=False
    for row in rows:
        if not row.get('executable'):continue
        name=row['target']['name']
        if name not in selections:
            # Cargo also builds the editor binary for these integration tests.
            # It remains audited/attributed, but is never selected or executed.
            require(package=='orr_editor' and selections in [
                {'ui_layer_order_acceptance':('test',True)},
                {'ui_layer_order_gpu_export':('test',True)}]
                and name=='orr_editor' and row['target']['kind']==['bin']
                and row['target']['src_path']==str(work/'crates/orr_editor/src/main.rs')
                and row['profile']['test'] is False,
                'unexpected executable selected package target')
            require(not auxiliary_seen and row['fresh'] is False,'duplicate/cached auxiliary executable')
            auxiliary_seen=True
            continue
        kind,is_test=selections[name]
        require(row['target']['kind']==[kind] and row['profile']['test'] is is_test,'wrong target kind/test profile')
        require(name not in selected and row['fresh'] is False,'duplicate/cached selected executable')
        selected[name]=row
    require(set(selected)==set(selections),'missing selected executable')
    return selected

def source_negative(work,pin):
    p=work/pin['file'];require(digest(p)==pin['sha256'],'negative source file differs')
    rows=p.read_text().splitlines();require(rows[pin['line']-1]==pin['text'],'negative source line differs')
    require(pin['column']==5 and pin['text'].startswith('    assert_eq!('),'negative source column differs')
    require(digest(work/pin['debug_definition_file'])==pin['debug_definition_sha256'],'Rust Debug definition changed')
    require(len(pin['diagnostics'])==2 and pin['diagnostics'][0].startswith('left: Document {') and pin['diagnostics'][1].startswith('right: Document {'),'negative exact document diagnostics absent')
    return {'path':pin['file'],'line':pin['line'],'column':pin['column'],'diagnostics':pin['diagnostics']}

def restored_identity(positive,restored):
    require(positive==restored and set(positive)=={'bytes','sha256'},'restored full ELF identity differs')

def rounded(value,block):
    require(type(value) is int and value>=0 and type(block) is int and block>0,'invalid allocation size')
    return ((value+block-1)//block)*block

def fixture_budget(identities,block=4096):
    """Hosted per-fixture copied-file arithmetic, using actual pinned ELF sizes.

    This is separate from local C's blocked admission. The host job has no local
    legacy allocation reserve to transfer. Pixel/copy allowances are unreduced.
    """
    require(set(identities)=={'generator','runtime','exporter','acceptance'},'incomplete actual producer sizes')
    copies={}
    for name,value in identities.items():
        require(set(value)=={'bytes','sha256'} and type(value['bytes']) is int and 64<=value['bytes']<=192*MIB and re.fullmatch('[0-9a-f]{64}',value['sha256']),'unmeasured/invalid full ELF identity')
        copies[name]=rounded(value['bytes'],block)*(2 if name=='runtime' else 1)
    parts={'pinned_elf_copies':sum(copies.values()),'project_package':16*MIB,
           'temporary_pngs':104*MIB,'logs_profile':16*MIB,'directory_allocation':64*block}
    return {'copies':copies,'parts':parts,'transient_bytes':sum(parts.values()),'derived_from_actual_pinned_sizes':True,'hard_aggregate_enforcement_proven':False}


def helper_result(text,stderr=''):
    require(text.count('ORR_LAYER_ORDER_ISOLATION_OK')==1,'missing/duplicate helper mask preflight')
    raw_running=[line for line in text.splitlines() if re.match(r'^\s*running\b',line)]
    require(raw_running==['running 1 test'],'helper running framing/cardinality differs')
    outcomes=re.findall(r'^test ([A-Za-z0-9_:]+) \.\.\. (ok|FAILED|ignored)$',text,re.M)
    raw_outcomes=[line for line in text.splitlines() if re.match(r'^\s*test\s+(?!result:)',line)]
    require(len(raw_outcomes)==len(outcomes) and all(re.fullmatch(r'test [A-Za-z0-9_:]+ \.\.\. (ok|FAILED|ignored)',line) for line in raw_outcomes),'malformed/unparsed helper outcome row')
    require(outcomes==[(GPU_HELPER,'ok')],'wrong/duplicate helper outcome')
    summaries=[line for line in text.splitlines() if re.match(r'^\s*test result:',line)]
    require(len(summaries)==1 and re.fullmatch(r'test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out;(?: finished in [0-9.]+s)?',summaries[0]),'helper terminal framing/counters differ')
    require('panicked at' not in text and 'panicked at' not in stderr,'helper stdout/stderr panic')
    require(not re.search(r'\b(?:skip|skipped|skipping|fallback)\b',text,re.I),'helper fallback')
    return {'exact_helper':GPU_HELPER,'count':1,'filtered':1}

def scan_fixture(root,cap,per_capture=8*MIB):
    """Quiescent complete evidence readback, not a live hard-write budget."""
    require(root.is_dir() and not root.is_symlink(),'missing ordinary fixture evidence')
    files={};allocated=root.stat().st_blocks*512;logical=0
    for path in root.iterdir():
        st=path.lstat();require(stat.S_ISREG(st.st_mode) and st.st_nlink==1,'unexpected fixture subdirectory/link/special file')
        limit=per_capture if path.suffix=='.png' else (4*MIB if path.name=='relocated-proof.json' else MIB)
        require(st.st_size<=limit,'fixture evidence per-file bound')
        logical+=st.st_size;allocated+=st.st_blocks*512;require(logical<=cap and allocated<=cap,'fixture evidence aggregate bound')
        if path.suffix=='.png':
            with path.open('rb') as stream:header=stream.read(33)
            require(len(header)==33 and header[:16]==b'\x89PNG\r\n\x1a\n\0\0\0\rIHDR','retained PNG header absent')
            require(struct.unpack('>IIBBBBB',header[16:29])==(1024,1024,8,6,0,0,0),'retained PNG dimensions/format differ')
            require(zlib.crc32(header[12:29])==struct.unpack('>I',header[29:33])[0],'retained PNG header CRC differs')
        files[path.name]={'bytes':st.st_size,'sha256':digest(path)}
    return files

def check_fixture(root,hide_roots,negative=False):
    files=scan_fixture(root,120*MIB)
    captures=CAPTURES[:1] if negative else CAPTURES
    labels=['00-generate','01-original-t0'] if negative else COMMANDS
    common={'producer-pins.json','source-masks.json','baseline-frames.json','original.ui.json','pixel-oracles.json'}
    expected=common|{name+'.png' for name in captures}|{name+suffix for name in labels for suffix in ['.stdout.txt','.stderr.txt','.status.txt','.transport.json']}
    if not negative:expected|={'complete.json','source-inventory.json','bundle-inventory.json','relocated-proof.json'}|{name+'.ui.json' for name in CAPTURES[1:5]}
    require(set(files)==expected,'missing/extra fixture evidence file')
    masks=json.loads((root/'source-masks.json').read_text());require(masks==[str(path) for path in hide_roots],'whole-workspace mask must remain complete')
    for name in labels:
        info=json.loads((root/(name+'.transport.json')).read_text())
        require(info=={'per_stream_cap':512*1024,'overflow':False,'expired':False,'errors':[],'descendant_cleanup_claim':False,'cleanup_owner':'outer reviewed controller'},'nested transport failed/claim changed')
        require((root/(name+'.status.txt')).read_text()=='exit status: 0','nested process failed')
        for suffix in ['.stdout.txt','.stderr.txt']:require(files[name+suffix]['bytes']<=512*1024,'nested stream exceeded cap')
        text=(root/(name+'.stdout.txt')).read_text();require(text.count('ORR_LAYER_ORDER_ISOLATION_OK')==1,'nested source-mask preflight absent/duplicate')
        if name in captures:require('software: true' in text,'software adapter not proven')
    if not negative:
        complete=json.loads((root/'complete.json').read_text())
        require(complete=={'software_gpu_captures':13,'order_states':['original','back','front','undo','redo'],'export_pixel_pairs':4,'frame_probes':[[0,False],[8,True],[16,True],[40,False]],'source_hidden_bundle_actions_and_full_frames':True,'native_window_or_physical_gpu_claim':False},'wrong GPU completion scope')
        helper_result((root/'07-relocated-proof.stdout.txt').read_text(),(root/'07-relocated-proof.stderr.txt').read_text())
        require(0<files['relocated-proof.json']['bytes']<4*MIB,'missing bounded nested helper proof')
    return {'files':files,'png_count':len(captures),'negative_is_order_only':negative,'whole_workspace_hidden':True}

def evidence_usage(root):
    """Logical AND allocation accounting; no suffix-based nested bypass."""
    evidence=0;archives=0;archive_logical=0;evidence_logical=0
    for parent,dirs,names in __import__('os').walk(root,followlinks=False):
        parent=pathlib.Path(parent);info=parent.lstat()
        require(stat.S_ISDIR(info.st_mode),'non-directory evidence parent')
        evidence+=info.st_blocks*512
        for name in dirs:require(not (parent/name).is_symlink(),'evidence symlink directory')
        for name in names:
            path=parent/name;info=path.lstat()
            require(stat.S_ISREG(info.st_mode) and info.st_nlink==1,'nonregular/multiply-linked evidence')
            if name.endswith('.elf.gz'):
                require(parent==root,'archive classification escaped top-level')
                archives+=info.st_blocks*512;archive_logical+=info.st_size
            else:
                require(path.suffix!='.gz','unrecognized compressed evidence bypass')
                evidence+=info.st_blocks*512;evidence_logical+=info.st_size
    return {'evidence':max(evidence,evidence_logical),'archives':max(archives,archive_logical)}
