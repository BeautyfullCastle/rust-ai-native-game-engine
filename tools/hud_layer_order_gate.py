#!/usr/bin/env python3
"""Mandatory exact-head hosted HUD application/GPU acceptance, Linux x86-64.

This gate uses job isolation plus observed resource floors and bounded owned
files. It does not claim a new cgroup, hard aggregate-memory limit or native-window
acceptance. Missing capabilities, resources or evidence are failures.
"""
import argparse,hashlib,json,os,pathlib,re,shutil,stat,sys,time,tomllib
sys.dont_write_bytecode=True
from hud_layer_order import contracts as c,evidence as tests,filesystem as fs,producer as audit
from hud_layer_order.owned_process import Runner,OwnershipUnknown,require,kernel_no_children
MIB=1024**2;GIB=1024**3
TEXT_CAP=128*MIB;CAPTURE_CAP=27*8*MIB;ARCHIVE_CAP=6*64*MIB
PANEL='crates/orr_editor/src/collect_ui_panel.rs'
MUTATION_FROM='self.error = self.reorder_selected(edge).err();'
MUTATION_TO='self.error = None; // test-only omitted reorder command'


def sha(path):return audit.digest(path)
def json_bytes(value):return (json.dumps(value,indent=2,sort_keys=True)+'\n').encode()

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--run',action='store_true');args=parser.parse_args()
    if not args.run:
        print('Hosted-only HUD gate; use --run in the mandatory GitHub Actions lane. No acceptance has run.');return 0
    require(not sys.flags.optimize and os.environ.get('GITHUB_ACTIONS')=='true','run in the admitted hosted lane without optimized Python')
    require(sys.platform=='linux' and os.uname().machine=='x86_64','Linux x86-64 gate required')
    expected=os.environ['ORR_HUD_EXPECTED_HEAD'];require(re.fullmatch('[0-9a-f]{40}',expected),'exact requested head required')
    work=pathlib.Path(os.environ['GITHUB_WORKSPACE']).resolve(strict=True)
    base=pathlib.Path(os.environ['RUNNER_TEMP']).resolve(strict=True)
    require(not base.is_relative_to(work) and not work.is_relative_to(base),'workspace and scratch overlap')
    label=os.environ['GITHUB_RUN_ID']+'-'+os.environ['GITHUB_RUN_ATTEMPT'];require(re.fullmatch('[0-9-]+',label),'unsafe run identity')
    evidence=base/('hud-layer-order-'+label);evidence.mkdir(mode=0o700)
    temp=pathlib.Path('/tmp')/('orr-hud-layer-order-'+label);temp.mkdir(mode=0o700)
    runtime_dir=temp/'xdg-runtime';runtime_dir.mkdir(mode=0o700)
    target=base/('hud-layer-order-target-'+label);target.mkdir()
    record={'state':'STARTING','requested_head':expected,'phase_a_original_head':'00159a57e63a0eb7e2112016c4db4e47044a7b61','phase_b_original_head':'7127c43e4487f3dd67b676ec5ee25c910019fb0e','earlier_passes_are_not_relabelled':True,'aggregate_memory_hard_limit':False,'native_window_physical_device_covered':False}
    def usage():return c.evidence_usage(evidence)
    def save(name,value):
        data=json_bytes(value);require(usage()['evidence']+len(data)+4096<=TEXT_CAP+CAPTURE_CAP,'evidence allocation cap')
        with (evidence/name).open('xb') as out:out.write(data)
    def observe():
        mem={}
        for line in pathlib.Path('/proc/meminfo').read_text().splitlines():
            if line.startswith(('MemTotal:','MemAvailable:')):
                key,value,unit=line.split();require(unit=='kB','memory observation units');mem[key[:-1]]=int(value)*1024
        values={'workspace_free':shutil.disk_usage(work).free,'tmp_free':shutil.disk_usage(temp).free,'evidence':usage(),
                'target':fs.observe_allocation(target),'fixture_tmp':fs.observe_allocation(temp),'host_memory_observed':mem,'aggregate_memory_hard_limit':False}
        require(min(values['workspace_free'],values['tmp_free'])>=GIB,'observed free-space floor breach')
        require(values['evidence']['evidence']<=TEXT_CAP+CAPTURE_CAP and values['evidence']['archives']<=ARCHIVE_CAP,'aggregate evidence/archive allocation exceeded')
        return values
    # A clean allowlist avoids ambient compiler/GPU/authentication overrides.
    env={k:os.environ[k] for k in ['HOME','USER','LOGNAME','PATH','LANG','LC_ALL','TZ','CARGO_HOME','RUSTUP_HOME'] if k in os.environ}
    env.update(CARGO_TERM_COLOR='never',CARGO_BUILD_JOBS='1',CARGO_INCREMENTAL='0',CARGO_NET_OFFLINE='true',CARGO_TARGET_DIR=str(target),RUST_BACKTRACE='1',PYTHONDONTWRITEBYTECODE='1',TMPDIR=str(temp),XDG_RUNTIME_DIR=str(runtime_dir),WGPU_BACKEND='vulkan',ORR_REQUIRE_GPU='1')
    runner=Runner(evidence,env,observe);save('started.json',record);save('owner-capability.json',runner.capability)
    mutated=False;original=None;active=None;baseline=None;source_names=[];archives={};fixtures={};tools={};tool_pins={};target_state={};retained_hashes={}
    try:
        git=pathlib.Path(shutil.which('git',path=env['PATH'])).resolve(strict=True)
        def run(name,argv,expected_exit=0,timeout=3600,extra_env=None):return runner.run(name,[str(x) for x in argv],work,expected_exit,timeout,extra_env)
        text,_=run('requested-head',[git,'rev-parse','HEAD'],timeout=60);require(text.strip()==expected,'checkout is not exact requested PR/push head')
        tree,_=run('requested-tree',[git,'rev-parse','HEAD^{tree}'],timeout=60);record['tree']=tree.strip()
        text,_=run('tracked-source',[git,'ls-files','-z'],timeout=60);source_names=text.rstrip('\0').split('\0')
        require(source_names and len(source_names)==len(set(source_names)),'empty/duplicate source inventory')
        baseline=fs.sources(work,source_names);active=baseline
        listing,_=run('requested-tree-leaves',[git,'ls-tree','-r','-z','--full-tree','HEAD'],timeout=60)
        record['source_tree_binding']=c.source_matches_tree(work,listing,baseline)
        require(fs.sources(work,source_names)==baseline,'source drift while binding Git tree')
        save('source-before.json',baseline);record['source_manifest_sha256']=c.object_digest(baseline)
        original=(work/PANEL).read_bytes()
        negative=json.loads((work/'tools/hud_layer_order/negative-oracle.json').read_text());negative_oracle=c.source_negative(work,negative)
        # Resolve actual toolchain binaries rather than invoking a hashed rustup
        # proxy under the wrong argv[0]. Every resolution/version is owner-guarded.
        rustup=pathlib.Path(shutil.which('rustup',path=env['PATH'])).resolve(strict=True)
        tool_pins[str(git)]=sha(git);tool_pins[str(rustup)]=sha(rustup)
        for name in ['cargo','rustc','cargo-clippy','clippy-driver']:
            text,_=run('resolve-'+name,[rustup,'which',name],timeout=60)
            path=pathlib.Path(text.strip());require(path.is_absolute() and path.resolve()==path and path.is_file(),'canonical real tool required')
            tools[name]=path;tool_pins[str(path)]=sha(path)
            version,_=run('version-'+name,[path,'-vV' if name=='rustc' else '-V'],timeout=60)
            save(name+'.tool.json',{'path':str(path),'sha256':tool_pins[str(path)],'version':version})
        require(len({p.parent for p in tools.values()})==1,'toolchain parents differ')
        env['PATH']=str(tools['cargo'].parent)+os.pathsep+env['PATH']
        infra={}
        for key,command in [('TIMEOUT','timeout'),('BWRAP','bwrap'),('SHA256SUM','sha256sum'),('SHELL','sh'),('LS','ls'),('DIRNAME','dirname')]:
            selected='/bin/sh' if key=='SHELL' else shutil.which(command,path=env['PATH'])
            require(selected is not None,'mandatory infrastructure missing: '+key)
            path=pathlib.Path(selected).resolve(strict=True);require(path.is_file() and os.access(path,os.X_OK),'invalid infrastructure')
            infra[key]={'path':str(path),'sha256':sha(path)};tool_pins[str(path)]=infra[key]['sha256']
        require((pathlib.Path(infra['DIRNAME']['path']).parent/'dirname').resolve()==pathlib.Path(infra['DIRNAME']['path']),'dirname alias differs')
        cargo_home=pathlib.Path(env.get('CARGO_HOME',str(pathlib.Path(env['HOME'])/'.cargo')))
        config=fs.config_set(work,cargo_home);save('configuration.json',{'environment':env,'cargo_configs':config,'infrastructure':infra,'tool_pins':tool_pins})
        require(tomllib.loads((work/'Cargo.toml').read_text())['profile']['release']=={'debug':1,'codegen-units':1,'lto':'thin'},'release profile changed')
        package_names={}
        for name in source_names:
            if name=='Cargo.toml' or name.endswith('/Cargo.toml'):
                doc=tomllib.loads((work/name).read_text())
                if 'package' in doc:package_names[name]=doc['package']['name']
        audit.ROOT=work;audit.TARGET=target;audit.EVIDENCE=evidence;audit.NAMES=source_names;audit.PACKAGES=package_names;audit.LOCAL_NAMES=set(package_names.values())
        audit.write_json=lambda path,value:save(path.name,value) if path.parent==evidence else require(False,'provenance path escaped')
        identities=fs.package_identities(work,source_names)
        target_state,target_start=fs.disk_inventory(target,hash_content=False);save('target-before.json',target_state)
        def fence():
            require(not runner.unknown,'UNKNOWN owner blocks source/admission actions')
            require(fs.sources(work,source_names)==active,'source input closure drift')
            require(fs.config_set(work,cargo_home)==config,'Cargo configuration drift')
            for name,digest in tool_pins.items():require(sha(pathlib.Path(name))==digest,'tool/infrastructure identity drift')
            require(not runner.pending,'interrupted gate')
        def target_fence():
            after,allocated=fs.disk_inventory(target,hash_content=False);require(after==target_state,'nonproducer target mutation')
            for name,digest in retained_hashes.items():require(sha(target/name)==digest,'retained producer hash changed')
            require(fs.disk_inventory(target,hash_content=False)==(after,allocated),'posthash target inventory drift')
        def command(name,argv,expected_exit=0,timeout=3600,extra_env=None,producer=False):
            fence();target_fence();observe()
            result=run(name,argv,expected_exit,timeout,extra_env);fence()
            if not producer:target_fence()
            return result
        def attribute(name,stdout):
            nonlocal target_state
            after,allocated=fs.disk_inventory(target,hash_content=False);rows,_=audit.producer_messages(name,stdout)
            receipt=json.loads((evidence/(name+'.transport.json')).read_text());provenance=json.loads((evidence/(name+'.provenance.json')).read_text())
            bins=fs.canonical_bin_proofs(provenance,target,int(receipt['started_ns']),int(receipt['finished_ns']))
            delta=fs.attribute_output_changes(target_state,after,rows,target,identities,bins)
            for path,value in delta['changed'].items():value['after_sha256']=sha(target/path);retained_hashes[path]=value['after_sha256']
            for path in delta['removed']:retained_hashes.pop(path,None)
            require(fs.disk_inventory(target,hash_content=False)==(after,allocated),'producer target drift during full hashes')
            save(name+'.output-delta.json',delta);target_state=after
        def produce(name,action,package,target_name=None,selections=None):
            fence();target_fence();require(shutil.disk_usage(work).free>=GIB+1536*MIB+(ARCHIVE_CAP-usage()['archives'])+(TEXT_CAP+CAPTURE_CAP-usage()['evidence']),'compile/link/remaining-evidence floor admission')
            features=c.EDITOR_FEATURES if package=='orr_editor' else c.TOOL_FEATURES
            audit.refresh(name,work,active);argv=c.command(action,package,target_name);argv[0]=str(tools['cargo'])
            stdout,stderr=command(name,argv,producer=True)
            rows=audit.audit_producer(name,stdout,stderr,work,package,features,action);attribute(name,stdout)
            if selections is None:return {}
            chosen=c.selected_roots(rows,work,package,selections,features);result={}
            for key,row in chosen.items():
                path=pathlib.Path(row['executable']);require(path.is_relative_to(target/'release') and path.resolve()==path,'selected executable escapes target')
                result[key]={'path':str(path),'identity':audit.elf(path,192*MIB),'producer':row,'source_manifest_sha256':c.object_digest(active)}
            save(name+'.artifacts.json',result);return result
        def inventory(label,artifact,names,ignored):
            path=pathlib.Path(artifact['path'])
            for kind,args,wanted in [('complete',['--list'],names),('ignored',['--ignored','--list'],ignored)]:
                audit.elf(path,192*MIB,artifact['identity']);text,_=command(label+'-'+kind,[path,*args],timeout=120)
                audit.elf(path,192*MIB,artifact['identity']);save(label+'-'+kind+'.verified.json',tests.inventory(text,wanted))
            artifact['names']=names
        def exact(label,artifact,name,gpu=False,negative_mode=False,extra_env=None):
            path=pathlib.Path(artifact['path']);audit.elf(path,192*MIB,artifact['identity']);require(name in artifact['names'],'test absent from same-producer inventory')
            argv=[path,name,'--exact','--show-output','--test-threads=1']+(['--ignored'] if gpu else [])
            text,stderr=command(label,argv,101 if negative_mode else 0,2700 if gpu else 300,extra_env)
            audit.elf(path,192*MIB,artifact['identity']);require('panicked at' not in stderr,'uncaptured or unrelated panic')
            save(label+'.verified.json',tests.result(text,[name],101 if negative_mode else 0,len(artifact['names'])-1,negative_oracle if negative_mode else None))
        def archive(label,artifact):
            fence();path=pathlib.Path(artifact['path']);audit.elf(path,192*MIB,artifact['identity'])
            require(len(archives)<6 and usage()['archives']+64*MIB<=ARCHIVE_CAP,'six archive capacity')
            receipt=tests.archive(path,evidence/(label+'.elf.gz'),compressed_cap=64*MIB);fs.require_archive_identity(receipt,artifact['identity'])
            save(label+'.archive.json',receipt);archives[label]={'identity':artifact['identity'],'receipt':receipt}
        def archive_fence(value):
            receipt=value['receipt'];path=pathlib.Path(receipt['archive']);require(sha(path)==receipt['archive_sha256'],'archive bytes drift')
            tests.readback(path,value['identity']['sha256'],value['identity']['bytes'])
        runtime=produce('cpu-producer','test','orr_editor','ui_layer_order_acceptance',{'ui_layer_order_acceptance':('test',True)})['ui_layer_order_acceptance']
        inventory('cpu',runtime,c.CPU_TESTS,[]);archive('cpu',runtime)
        for index,name in enumerate(c.CPU_TESTS):exact('cpu-exact-'+str(index),runtime,name)
        produce('combined-strict','clippy','orr_editor')
        production=produce('production-tools','build','orr_sample',selections={name:('bin',False) for name in c.TOOLS})
        for name,value in production.items():archive(name,value)
        def gpu_producer(label):
            item=produce(label,'test','orr_editor','ui_layer_order_gpu_export',{'ui_layer_order_gpu_export':('test',True)})['ui_layer_order_gpu_export']
            inventory(label,item,c.GPU_TESTS,c.GPU_TESTS);return item
        def gpu(label,artifact,negative_mode=False):
            sizes={'generator':production['orr_new_arena']['identity'],'runtime':production['collect_dodge']['identity'],'exporter':production['orr_export_collect']['identity'],'acceptance':artifact['identity']}
            block=os.statvfs(temp).f_frsize;copy_budget=c.fixture_budget(sizes,block)
            # One physical pool gets one floor; extra needed copied data is always
            # charged in addition to all remaining retained archives/evidence.
            transient=copy_budget['transient_bytes']
            remaining=(ARCHIVE_CAP-usage()['archives'])+(TEXT_CAP+CAPTURE_CAP-usage()['evidence'])
            shared=temp.stat().st_dev==evidence.stat().st_dev
            need_tmp=GIB+transient+(remaining if shared else 0)
            require(shutil.disk_usage(temp).free>=need_tmp,'actual ELF-copy fixture admission failed')
            if not shared:require(shutil.disk_usage(evidence).free>=GIB+remaining,'separate evidence pool unavailable')
            save(label+'.resource-admission.json',{'pinned_identities':sizes,'copy_budget':copy_budget,'shared_physical_pool':shared,'required_tmp_free':need_tmp,'memory_peak_preproved':False,'resource_limits':'existing hosted job; observed floors and bounded fixture-owned files'})
            root=evidence/(label+'-fixture');require(not root.exists(),'fresh fixture evidence required')
            gpu_env=dict(env);gpu_env.update(ORR_LAYER_ORDER_HIDE_ROOT=str(work),ORR_LAYER_ORDER_EVIDENCE=str(root),ORR_LAYER_ORDER_ACCEPTANCE_SHA256=artifact['identity']['sha256'])
            for name,value in infra.items():gpu_env['ORR_LAYER_ORDER_'+name+'_BIN']=value['path'];gpu_env['ORR_LAYER_ORDER_'+name+'_SHA256']=value['sha256']
            for name,path_key,hash_key in [('orr_new_arena','ORR_NEW_ARENA_BIN','GENERATOR'),('collect_dodge','ORR_COLLECT_RUNTIME','RUNTIME'),('orr_export_collect','ORR_COLLECT_EXPORTER','EXPORTER')]:
                value=production[name];audit.elf(pathlib.Path(value['path']),192*MIB,value['identity']);gpu_env[path_key]=value['path'];gpu_env['ORR_LAYER_ORDER_'+hash_key+'_SHA256']=value['identity']['sha256']
            exact(label,artifact,c.GPU_MAIN,True,negative_mode,gpu_env)
            masks=sorted({work,*[pathlib.Path(value['path']).parent for value in production.values()],pathlib.Path(artifact['path']).parent})
            masks=[path for path in masks if not any(path!=other and path.is_relative_to(other) for other in masks)]
            receipt=c.check_fixture(root,masks,negative_mode);fixtures[str(root)]={'negative':negative_mode,'masks':masks,'receipt':receipt};save(label+'.fixture.json',receipt)
            supplied=json.loads((root/'producer-pins.json').read_text())
            require(set(supplied)==set(c.INFRA)|{'ORR_NEW_ARENA_BIN','ORR_COLLECT_RUNTIME','ORR_COLLECT_EXPORTER','acceptance'},'unexpected fixture identity entries')
            for key,value in infra.items():require(supplied[key]==value,'fixture infrastructure attribution differs')
            for name,key in [('orr_new_arena','ORR_NEW_ARENA_BIN'),('collect_dodge','ORR_COLLECT_RUNTIME'),('orr_export_collect','ORR_COLLECT_EXPORTER')]:
                require(supplied[key]['source']==production[name]['path'] and supplied[key]['sha256']==production[name]['identity']['sha256'],'fixture tool attribution differs')
            require(supplied['acceptance']['source']==artifact['path'] and supplied['acceptance']['sha256']==artifact['identity']['sha256'],'copied helper differs from selected ELF')
        positive=gpu_producer('gpu-positive-producer');archive('gpu-positive',positive);gpu('gpu-positive',positive)
        fence();target_fence();text=(work/PANEL).read_text();require(text.count(MUTATION_FROM)==1,'negative widget site not unique')
        mutated=True;(work/PANEL).write_text(text.replace(MUTATION_FROM,MUTATION_TO));active=fs.sources(work,source_names)
        require({name for name in baseline if baseline[name]!=active[name]}=={PANEL},'negative changed more than the widget handler');save('negative-source.json',active)
        negative_artifact=gpu_producer('gpu-negative-producer');require(negative_artifact['identity']!=positive['identity'],'negative did not change full ELF')
        archive('gpu-negative',negative_artifact);gpu('gpu-negative',negative_artifact,True)
        fence();(work/PANEL).write_bytes(original);active=baseline;fence();mutated=False
        restored=gpu_producer('gpu-restored-producer');c.restored_identity(positive['identity'],restored['identity']);archive_fence(archives['gpu-positive'])
        save('gpu-restored.archive-alias.json',{'full_elf':restored['identity'],'positive_archive':archives['gpu-positive'],'full_readback':True});gpu('gpu-restored',restored)
        require(len(archives)==6,'missing archive roles')
        for value in archives.values():archive_fence(value)
        for path,value in fixtures.items():require(c.check_fixture(pathlib.Path(path),value['masks'],value['negative'])==value['receipt'],'retained fixture bytes changed')
        fence();target_fence();head,_=command('final-head',[git,'rev-parse','HEAD'],timeout=60);require(head.strip()==expected,'HEAD drift')
        tree_after,_=command('final-tree',[git,'rev-parse','HEAD^{tree}'],timeout=60);require(tree_after.strip()==record['tree'],'tree drift')
        require(kernel_no_children(),'terminal cleanup not empty')
        record.update(state='PASS_EXACT_HEAD_COMBINED_HUD_SCOPE',final_source_manifest_sha256=c.object_digest(fs.sources(work,source_names)),source_restored=True,negative_is_order_assertion_not_pixel_negative=True,kernel_ECHILD=True,final_resources=observe())
    except BaseException as error:
        record.update(state='FAIL_OWNERSHIP_UNKNOWN' if runner.unknown else 'FAIL',error=repr(error),ownership_unknown=runner.unknown);raise
    finally:
        if mutated and not runner.unknown:
            (work/PANEL).write_bytes(original)
            require(fs.sources(work,source_names)==baseline,'failed negative restoration')
            record['source_restored_after_failure']=True
        elif mutated:record['source_restoration_blocked_by_unknown_ownership']=True
        save('terminal.json',record)
    return 0

if __name__=='__main__':raise SystemExit(main())
