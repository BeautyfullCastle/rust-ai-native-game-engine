"""Exact approved 0f2a8dc3 producer helpers (AST-identical to f775); extracted without executing source tool.
Globals ROOT/TARGET/EVIDENCE/NAMES/PACKAGES/LOCAL_NAMES are bound by controller.
No feature-source, parser policy or original helper was edited.
"""
import hashlib,json,os,re,shlex,stat,struct,time
from pathlib import Path
MIB=1024*1024

def require(condition, message):
    if not condition:
        raise RuntimeError(message)

def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(MIB), b''):
            h.update(chunk)
    return h.hexdigest()

def write_json(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')

def source_inventory(root, names):
    entries = {}
    for name in names:
        path = root / name
        require(path.is_file() and not path.is_symlink(), f'non-regular tracked input: {path}')
        require(path.resolve().is_relative_to(root), f'input escapes source root: {path}')
        info = path.stat()
        entries[name] = {'sha256': digest(path), 'bytes': info.st_size, 'mode': stat.S_IMODE(info.st_mode)}
    return entries

def refresh(name, root, baseline):
    require(source_inventory(root, NAMES) == baseline, f'{name}: source closure changed before refresh')
    cached = []
    fingerprint = TARGET / 'release/.fingerprint'
    if fingerprint.exists():
        for entry in fingerprint.iterdir():
            if any(entry.name.startswith(package + '-') for package in LOCAL_NAMES):
                cached.extend(p.stat().st_mtime_ns for p in entry.rglob('*') if p.is_file())
    cached_max = max(cached, default=0)
    now = time.time_ns()
    require(cached_max < now, f'{name}: local cached fingerprints are not older than the current clock')
    records = []
    for relative in NAMES:
        path = root / relative
        prior = path.stat().st_mtime_ns
        require(prior <= now, f'{name}: future source mtime: {relative}')
        os.utime(path, ns=(now, now), follow_symlinks=False)
        observed = path.stat().st_mtime_ns
        require(cached_max < observed <= time.time_ns(), f'{name}: refresh did not exceed cached local fingerprints')
        records.append({'path': relative, 'prior_mtime_ns': prior, 'mtime_ns': observed,
                        'sha256': baseline[relative]['sha256']})
    require(source_inventory(root, NAMES) == baseline, f'{name}: refresh changed source bytes or modes')
    write_json(EVIDENCE / f'{name}.refresh.json', {
        'root': str(root), 'scope': 'complete tracked local workspace, dependency, fixture and config inputs',
        'registry_and_other_worktrees_untouched': True, 'cached_max_mtime_ns': cached_max,
        'clock_ns': now, 'verified_at_ns': time.time_ns(), 'files': records})

def options(tokens, name):
    return [tokens[i + 1] for i, token in enumerate(tokens[:-1]) if token == name]

def running_commands(stderr):
    # Cargo shell-quotes environment values, including literal newlines/backticks.
    # A trailing backtick closes a record only outside an unfinished shell quote.
    pending = None
    start = None
    for number, line in enumerate(stderr.splitlines(), 1):
        match = re.match(r'^\s*Running `', line)
        if match:
            require(pending is None, f'nested verbose command at line {number}; unfinished line {start}')
            pending = line[match.end():]
            start = number
        elif pending is not None:
            pending += '\n' + line
        else:
            continue
        closed = pending.endswith('`')
        try:
            tokens = shlex.split(pending[:-1] if closed else pending)
        except ValueError as error:
            require(str(error) == 'No closing quotation',
                    f'malformed verbose command at line {start}: {error}')
            continue
        require(closed, f'missing verbose command delimiter at line {start}')
        require(tokens, f'empty verbose command at line {start}')
        yield tokens
        pending = None
    require(pending is None, f'unterminated verbose command at line {start}')

def commands(stderr, root):
    result = []
    for tokens in running_commands(stderr):
        if '--crate-name' not in tokens:
            continue
        settings = dict(t.split('=', 1) for t in tokens if re.match(r'^CARGO_[A-Z_]+=.*', t))
        if settings.get('CARGO_PKG_NAME') not in LOCAL_NAMES:
            continue
        manifest = settings.get('CARGO_MANIFEST_PATH')
        if manifest is None:
            manifest = str(Path(settings['CARGO_MANIFEST_DIR']) / 'Cargo.toml')
        manifest = Path(manifest).resolve()
        require(manifest.is_relative_to(root), 'verbose local compile belongs to another worktree')
        sources = [str((root / token).resolve()) for token in tokens
                   if token.endswith('.rs') and '=' not in token]
        require(len(sources) == 1, f'ambiguous verbose source: {sources}')
        result.append({'tokens': tokens, 'manifest': str(manifest), 'source': sources[0],
                       'crate': options(tokens, '--crate-name')[0], 'test': '--test' in tokens,
                       'features': sorted(x[len('feature="'):-1] for x in options(tokens, '--cfg')
                                          if x.startswith('feature="') and x.endswith('"'))})
    require(result, 'producer has no verbose local rustc/clippy invocations')
    return result

def producer_messages(name, stdout):
    # Cargo -vv forwards build-script output alongside its JSON messages.
    # Only the anchored Cargo verbose prefix is accepted as unrelated output;
    # its payload is never parsed, even if it resembles a successful build.
    verbose = re.compile(r'\[[A-Za-z0-9_][A-Za-z0-9_-]* '
                         r'[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?'
                         r'(?:\+[0-9A-Za-z.-]+)?\] [^\r\n]*')
    def unique_object(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                raise ValueError('duplicate JSON object key')
            value[key] = item
        return value

    def invalid_constant(value):
        raise ValueError('non-finite JSON constant')

    rows = []
    counts = {'json_objects': 0, 'verbose_build_script_lines': 0, 'blank_lines': 0}
    verbose_digest = hashlib.sha256()
    for number, raw_line in enumerate(stdout.splitlines(keepends=True), 1):
        line = raw_line.rstrip('\r\n')
        if not line.strip():
            counts['blank_lines'] += 1
        elif line.lstrip().startswith('{'):
            try:
                row = json.loads(line, object_pairs_hook=unique_object, parse_constant=invalid_constant)
            except ValueError as error:
                raise AssertionError(f'{name}: malformed producer JSON at line {number}') from error
            require(isinstance(row, dict), f'{name}: non-object producer JSON at line {number}')
            rows.append(row)
            counts['json_objects'] += 1
        else:
            require(verbose.fullmatch(line) is not None,
                    f'{name}: unrecognized producer output at line {number}')
            counts['verbose_build_script_lines'] += 1
            verbose_digest.update(raw_line.encode('utf-8'))
    return rows, {'counts': counts, 'verbose_build_script_sha256': verbose_digest.hexdigest()}

def audit_producer(name, stdout, stderr, root, package, features, action):
    rows, stream_summary = producer_messages(name, stdout)
    require([row for row in rows if row.get('reason') == 'build-finished'] ==
            [{'reason': 'build-finished', 'success': True}], f'{name}: missing unique successful JSON build-finished')
    local = [row for row in rows if row.get('reason') == 'compiler-artifact'
             and row['package_id'].startswith('path+')]
    require(local, f'{name}: missing local compiler artifacts')
    invocations = commands(stderr, root)
    emitted = {str(Path(filename).resolve()): row for row in local for filename in row['filenames']}
    proof = []
    for row in local:
        manifest = Path(row['manifest_path']).resolve()
        require(manifest.is_relative_to(root) and str(manifest.relative_to(root)) in PACKAGES,
                f'{name}: local artifact outside complete tracked source closure')
        require(row.get('fresh') is False, f'{name}: cached local artifact is not fresh provenance: {manifest}')
        matches = [entry for entry in invocations if entry['manifest'] == str(manifest)
                   and entry['source'] == str(Path(row['target']['src_path']).resolve())
                   and entry['crate'] == row['target']['name'].replace('-', '_')
                   and entry['test'] == row['profile']['test']
                   and entry['features'] == sorted(row['features'])]
        require(matches, f'{name}: no exact verbose source/test/feature identity for {row["target"]["name"]}')
        externs = []
        for match in matches:
            for argument in options(match['tokens'], '--extern'):
                dependency, sep, filename = argument.partition('=')
                if sep and (dependency in LOCAL_NAMES or str(Path(filename).resolve()) in emitted):
                    path = Path(filename).resolve(strict=True)
                    require(str(path) in emitted, f'{name}: local extern has no producer JSON artifact: {argument}')
                    require(path.is_file() and not path.is_symlink(), f'{name}: invalid local extern')
                    externs.append({'dependency': dependency, 'path': str(path),
                                    'bytes': path.stat().st_size, 'sha256': digest(path)})
        proof.append({'artifact': row, 'verbose': matches, 'local_externs': externs})
    roots = [row for row in local if Path(row['manifest_path']).resolve() == root / f'crates/{package}/Cargo.toml']
    require(roots, f'{name}: selected package has no artifacts')
    for row in roots:
        require(sorted(row['features']) == sorted(features), f'{name}: wrong selected feature closure: {row["features"]}')
        expected = {'opt_level': '3', 'debuginfo': 1, 'debug_assertions': False,
                    'overflow_checks': False, 'test': row['profile']['test']}
        require(row['profile'] == expected, f'{name}: exact release profile changed: {row["profile"]}')
        matching = [entry for entry in invocations if entry['manifest'] == str(Path(row['manifest_path']).resolve())
                    and entry['source'] == str(Path(row['target']['src_path']).resolve())
                    and entry['test'] == row['profile']['test'] and entry['features'] == sorted(row['features'])]
        require(all({'opt-level=3', 'codegen-units=1', 'debuginfo=1'} <= set(options(entry['tokens'], '-C'))
                    for entry in matching), f'{name}: selected verbose release flags differ')
        if row.get('executable'):
            require(all('lto=thin' in options(entry['tokens'], '-C') for entry in matching),
                    f'{name}: selected executable was not linked with declared thin LTO')
    write_json(EVIDENCE / f'{name}.provenance.json', {
        'action': action, 'package': package, 'expected_features': features,
        'root': str(root), 'local_artifacts': proof, 'selected_artifacts': roots,
        'producer_stdout_stream': stream_summary,
        'producer_stdout_sha256': digest(EVIDENCE / f'{name}.stdout'),
        'producer_stderr_sha256': digest(EVIDENCE / f'{name}.stderr')})
    return roots

def elf(path, cap, expected=None):
    require(path.is_file() and not path.is_symlink(), f'not a regular ELF file: {path}')
    info = path.stat()
    require(64 <= info.st_size <= cap and info.st_mode & 0o111, f'executable cap/mode mismatch: {path}')
    with path.open('rb') as stream:
        header = stream.read(64)
    require(header[:7] == b'\x7fELF\x02\x01\x01' and struct.unpack_from('<H', header, 16)[0] in (2, 3)
            and struct.unpack_from('<H', header, 18)[0] == 62
            and struct.unpack_from('<H', header, 52)[0] == 64, f'not Linux x86_64 full ELF: {path}')
    measured = {'bytes': info.st_size, 'sha256': digest(path)}
    require(expected is None or measured == expected, f'full executable bytes changed: {path}')
    return measured
