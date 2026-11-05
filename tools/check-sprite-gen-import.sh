#!/usr/bin/env bash
# Focused, exact-source CPU/real-CLI gate. No upstream generator or GPU execution.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
[[ "$(uname -s)" == Linux ]] || { echo 'Linux required' >&2; exit 2; }
export CARGO_INCREMENTAL=0 CARGO_TERM_COLOR=never PYTHONDONTWRITEBYTECODE=1
EVIDENCE=${EVIDENCE:-$(mktemp -d /tmp/orr-sprite-gen-evidence.XXXXXXXX)}
mkdir -p "$EVIDENCE"
EVIDENCE=$(realpath "$EVIDENCE")
export EVIDENCE
if [[ -n "${EXPECTED_HEAD:-}" ]]; then
  [[ $(git rev-parse HEAD) == "$EXPECTED_HEAD" ]]
  git diff --exit-code
  git diff --cached --exit-code
fi
git rev-parse HEAD HEAD^{tree} > "$EVIDENCE/source.txt"
rustc --version --verbose > "$EVIDENCE/rustc.txt"
cargo --version > "$EVIDENCE/cargo.txt"
python3 - "$EVIDENCE" <<'PY'
import hashlib,json,subprocess,sys
from pathlib import Path
files=subprocess.check_output(['git','ls-files','-z']).decode().split('\0')
Path(sys.argv[1],'source-sha256.json').write_text(json.dumps({p:hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in files if p},indent=2)+'\n')
PY
run() { local label=$1; shift; printf '%q ' "$@" > "$EVIDENCE/$label.command"; printf '\n' >> "$EVIDENCE/$label.command"; "$@" 2>&1 | tee "$EVIDENCE/$label.log"; }
run sprite cargo test --locked --release -p orr_sprite --all-features
run package cargo test --locked --release -p orr_package --features sprite-authoring --lib
cargo build --locked --release -p orr_package --features sprite-authoring --bin orr_pkg --message-format=json > "$EVIDENCE/package-build.jsonl" 2> "$EVIDENCE/package-build.log"
cargo test --locked --release -p orr_sample --features collect-sprites,project-create --test sprite_gen_playback --no-run --message-format=json > "$EVIDENCE/playback-build.jsonl" 2> "$EVIDENCE/playback-build.log"
python3 - "$EVIDENCE" <<'PY'
import json,shutil,sys
from pathlib import Path
root=Path(sys.argv[1]); (root/'bin').mkdir(exist_ok=True)
for log,name,kind,test,dest in [('package-build','orr_pkg','bin',False,'orr_pkg'),('playback-build','sprite_gen_playback','test',True,'sprite_gen_playback')]:
    records=[json.loads(line) for line in (root/(log+'.jsonl')).read_text().splitlines()]
    selected=[r for r in records if r.get('reason')=='compiler-artifact' and r.get('target',{}).get('name')==name and r['target']['kind']==[kind] and r.get('profile',{}).get('test') is test and r.get('executable')]
    assert len(selected)==1, selected
    r=selected[0]
    expected=('sprite-authoring' if name=='orr_pkg' else 'collect-sprites')
    assert expected in r['features']
    shutil.copy2(r['executable'],root/'bin'/dest)
PY
export ORR_SPRITE_GEN_PACKAGE_BIN="$EVIDENCE/bin/orr_pkg"
run playback "$EVIDENCE/bin/sprite_gen_playback" --test-threads=1
run cli "$EVIDENCE/bin/sprite_gen_playback" --ignored --test-threads=1
python3 - "$EVIDENCE" <<'PY'
import re,sys
from pathlib import Path
root=Path(sys.argv[1])
expected={'playback':(3,1),'cli':(1,0)}
for label,(passed,ignored) in expected.items():
    text=(root/(label+'.log')).read_text()
    found=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',text)
    assert found==[(str(passed),'0',str(ignored))],(label,found)
    cases=re.findall(r'^test (\S+) \.\.\. ok$',text,re.M)
    assert len(cases)==passed and len(set(cases))==passed,(label,cases)
PY
run existing-collect cargo test --locked --release -p orr_sample --features collect-sprites,project-export --test collect_sprites
run strict-sprite cargo clippy --locked --release -p orr_sprite --all-features --all-targets -- -D warnings
run strict-package cargo clippy --locked --release -p orr_package --features sprite-authoring --all-targets -- -D warnings
run strict-playback cargo clippy --locked --release -p orr_sample --features collect-sprites,project-create --test sprite_gen_playback -- -D warnings
run feature-off cargo check --locked --release -p orr_package --no-default-features --bin orr_pkg
run editor-deps cargo test --locked --release -p orr_editor --test deps
python3 - "$EVIDENCE" <<'PY'
import hashlib,json,sys
from pathlib import Path
root=Path(sys.argv[1]); before=json.loads((root/'source-sha256.json').read_text())
assert all(hashlib.sha256(Path(p).read_bytes()).hexdigest()==digest for p,digest in before.items()),'source changed during gate'
(root/'artifact-sha256.json').write_text(json.dumps({str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(root.rglob('*')) if p.is_file() and p.name!='artifact-sha256.json'},indent=2)+'\n')
PY
printf 'Sprite-gen import CPU/CLI checks passed; evidence: %s\n' "$EVIDENCE"
printf '%s\n' 'No upstream generation, actual window/browser/GPU or physical-device acceptance claimed.'
