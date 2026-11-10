"""Generated Room -> explicit attachment -> source-hidden exported CPU reopen.
Pass trusted binaries built together from this checkout with room-controls-attach,project-create.
"""
from pathlib import Path
import subprocess, tempfile, json, hashlib, sys
import argparse
ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description="Explicit trusted same-source Linux x86_64 producers; CPU only, no builds/downloads")
parser.add_argument("--binary-dir", required=True, type=Path)
BIN = parser.parse_args().binary_dir.resolve()
def run(*args):
    result=subprocess.run([str(a) for a in args],capture_output=True,text=True,timeout=90)
    print('$',*args, '\n', result.stdout, result.stderr)
    assert result.returncode==0, result.returncode
    return result.stdout
with tempfile.TemporaryDirectory() as tmp:
    root=Path(tmp); project=root/'한글 Room'
    run(BIN/'orr_new_arena','--output',project,'--template','room-escape-3d-v1','--seed','attach-integration')
    old=(project/'orr.project.json').read_bytes()
    controls=project/'custom.json'
    run(sys.executable,ROOT/'tools/create_room_controls.py','--output',controls,'--move-left','j','--move-right','l','--move-forward','i','--move-backward','k','--interact','o')
    exact=controls.read_bytes()
    run(BIN/'orr_attach_room_controls','--help')
    run(BIN/'orr_attach_room_controls','--project',project,'--controls','custom.json')
    committed=(project/'orr.project.json').read_bytes()
    for args in [[], ['--project',str(project),'--controls','custom.json'], ['--project',str(project),'--controls','../custom.json']]:
        refused=subprocess.run([str(BIN/'orr_attach_room_controls'),*args],capture_output=True,text=True,timeout=30)
        assert refused.returncode != 0
        assert (project/'orr.project.json').read_bytes()==committed
        assert (project/'orr.project.before-controls.json').read_bytes()==old
    assert (project/'orr.project.before-controls.json').read_bytes()==old
    assert controls.read_bytes()==exact
    expected=json.loads(old); expected['entry']['controls']='custom.json'
    assert json.loads((project/'orr.project.json').read_bytes())==expected
    # Author a distinct revision; the original sidecar and first backup stay intact.
    revised=project/'controls-v2.json'
    run(sys.executable,ROOT/'tools/create_room_controls.py','--output',revised,'--move-left','a','--move-right','d','--move-forward','w','--move-backward','s','--interact','e')
    revised_exact=revised.read_bytes()
    revision_backup=project/'orr.project.before-controls-v2.json'
    for flags in [
        ['--expect-controls','custom.json'],
        ['--backup',revision_backup.name],
        ['--expect-controls','CUSTOM.json','--backup',revision_backup.name],
        ['--expect-controls','custom.json','--backup','CUSTOM.json'],
        ['--expect-controls','custom.json','--backup','CONTROLS-V2.json'],
        ['--expect-controls','custom.json','--backup','../backup.json'],
    ]:
        refused=subprocess.run([str(BIN/'orr_attach_room_controls'),'--project',str(project),'--controls',revised.name,*flags],capture_output=True,text=True,timeout=30)
        assert refused.returncode==1, refused
        assert (project/'orr.project.json').read_bytes()==committed
        assert not revision_backup.exists()
    run(BIN/'orr_attach_room_controls','--project',project,'--controls',revised.name,'--expect-controls','custom.json','--backup',revision_backup.name)
    assert revision_backup.read_bytes()==committed
    assert controls.read_bytes()==exact
    assert revised.read_bytes()==revised_exact
    assert (project/'orr.project.before-controls.json').read_bytes()==old
    expected['entry']['controls']=revised.name
    revised_manifest=(project/'orr.project.json').read_bytes()
    assert json.loads(revised_manifest)==expected
    retry=subprocess.run([str(BIN/'orr_attach_room_controls'),'--project',str(project),'--controls',revised.name,'--expect-controls','custom.json','--backup','retry.json'],capture_output=True,text=True,timeout=30)
    assert retry.returncode==1
    assert not (project/'retry.json').exists()
    assert (project/'orr.project.json').read_bytes()==revised_manifest
    assert revision_backup.read_bytes()==committed
    runtime=BIN/'room_escape'
    before=run(runtime,'--project',project,'--headless','--ticks','42','--hold','left,right,up,down,interact','--check-model-poses')
    exported=root/'exported'
    digest=hashlib.sha256(runtime.read_bytes()).hexdigest()
    run(BIN/'orr_export_room','--project',project,'--runtime',runtime,'--runtime-sha256',digest,'--output',exported,'--trusted-runtime')
    assert (exported/'project/controls-v2.json').read_bytes()==revised_exact
    assert not (exported/'project/custom.json').exists()
    assert not (exported/'project'/revision_backup.name).exists()
    project.rename(root/'source-hidden')
    after=run(exported/'run-room-escape','--headless','--ticks','42','--hold','left,right,up,down,interact','--check-model-poses')
    assert before==after
    print('PASS: real generated project, authored custom controls, explicit attachment/revision, stale-reference refusal, byte-exact old/new sidecars and version backups, CPU reopen/export/source-hidden launcher identical. No physical input/GPU coverage.')
    for name in ('orr_attach_room_controls','orr_new_arena','room_escape','orr_export_room'):
        print(name,hashlib.sha256((BIN/name).read_bytes()).hexdigest())
