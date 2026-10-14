"""Strict libtest and streaming archive contracts, reviewed source extraction."""
import gzip,hashlib,json,os,pathlib,re,stat
MIB=1024**2

def require(condition, message):
    if not condition: raise ValueError(message)

def digest(path):
    h=hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda:stream.read(MIB),b''):h.update(chunk)
    return h.hexdigest()

def inventory(text, expected=None):
    names=[]; summaries=[]
    for line in text.splitlines():
        if not line.strip():continue
        m=re.fullmatch(r'([A-Za-z0-9_:]+): test',line)
        if m:names.append(m.group(1));continue
        m=re.fullmatch(r'(\d+) tests?, 0 benchmarks?',line)
        if m:summaries.append(int(m.group(1)));continue
        raise ValueError('unexpected inventory line: '+line)
    require(len(names)==len(set(names)), 'duplicate test identity')
    if expected is not None:require(sorted(names)==sorted(expected), 'exact inventory mismatch')
    require(summaries==[len(names)], 'missing/duplicate/wrong inventory summary')
    return {'names':names,'count':len(names)}

def result(text, expected, exit_code, filtered, negative=None):
    require(isinstance(filtered,int) and filtered>=0, 'same-producer filtered count required')
    require(exit_code==(101 if negative else 0), 'unexpected actual process exit')
    raw_running=[line for line in text.splitlines() if re.match(r'^\s*running\b',line)]
    require(raw_running==[f'running {len(expected)} test'+('' if len(expected)==1 else 's')], 'running cardinality differs')
    outcomes=re.findall(r'^test ([A-Za-z0-9_:]+) \.\.\. (ok|FAILED|ignored)$',text,re.M)
    raw_outcomes=[line for line in text.splitlines() if re.match(r'^\s*test\s+(?!result:)',line)]
    require(len(raw_outcomes)==len(outcomes) and all(re.fullmatch(r'test [A-Za-z0-9_:]+ \.\.\. (ok|FAILED|ignored)',line) for line in raw_outcomes), 'malformed/unparsed test outcome row')
    require(len(outcomes)==len(expected), 'wrong count or duplicate outcomes')
    require(sorted(n for n,_ in outcomes)==sorted(expected), 'wrong exact test outcomes')
    require(all(status==('FAILED' if negative else 'ok') for _,status in outcomes), 'wrong test status')
    raw_summaries=[line for line in text.splitlines() if re.match(r'^\s*test result:',line)]
    require(len(raw_summaries)==1, 'missing/duplicate raw terminal summary')
    match=re.fullmatch(r'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;(?: finished in [0-9.]+s)?',raw_summaries[0])
    require(match is not None,'malformed terminal summary')
    state,passed,failed,ignored,measured,observed_filtered=match.groups()
    require((state,int(passed),int(failed),int(ignored),int(measured),int(observed_filtered))==
        (('FAILED',0,len(expected),0,0,filtered) if negative else ('ok',len(expected),0,0,0,filtered)), 'terminal counts disagree')
    label='failures' if negative else 'successes'
    blocks=list(re.finditer(r'^'+label+r':\s*$',text,re.M))
    require(blocks,'missing named outcome block')
    tail=text[blocks[-1].end():text.index(raw_summaries[0])]
    names=[line.strip() for line in tail.splitlines() if line.strip()]
    require(sorted(names)==sorted(expected),'named outcome block differs')
    panics=[line for line in text.splitlines() if 'panicked at' in line]
    if negative:
        require(len(expected)==1,'negative must target exactly one test')
        require(len(panics)==1,'missing/extra/unrelated panic')
        pattern=r"thread '"+re.escape(expected[0])+r"'(?: \(\d+\))? panicked at "+re.escape(negative['path'])+r':'+str(negative['line'])+':'+str(negative['column'])+':'
        require(re.fullmatch(pattern,panics[0]) is not None,'negative panic attribution differs')
        require(text.count('---- '+expected[0]+' stdout ----')==1,'missing/duplicate target failure block')
        require(len(re.findall(r'^---- .* stdout ----$',text,re.M))==1,'unrelated captured failure block')
        assertion='assertion `left == right` failed'
        require(text.count(assertion)==1,'wrong/duplicate negative assertion')
        detail=text.split(panics[0],1)[1].splitlines()
        nonblank=[line.strip() for line in detail if line.strip()]
        require(nonblank[:3]==[assertion,*negative['diagnostics']],'source-pinned assertion operands differ')
    else:require(not panics,'unexpected panic in positive execution')
    require(not re.search(r'\b(?:skipped|skipping|fallback execution)\b',text,re.I), 'skip/fallback marker')
    return {'outcomes':outcomes,'summary':match.groups(),'actual_exit':exit_code,'filtered_from_same_producer_inventory':filtered}

def raw_identity(path):
    st=path.lstat()
    require(stat.S_ISREG(st.st_mode), 'input must be regular and not a symlink')
    return st.st_dev,st.st_ino,st.st_size,st.st_mtime_ns

class BoundedWriter:
    def __init__(self,stream,limit):self.stream=stream;self.limit=limit
    def write(self,data):
        require(self.stream.tell()+len(data)<=self.limit,'archive cap before write; partial retained')
        return self.stream.write(data)
    def tell(self):return self.stream.tell()
    def flush(self):return self.stream.flush()

def bounded_hash(source,limit):
    h=hashlib.sha256();count=0
    with source.open('rb') as stream:
        for data in iter(lambda:stream.read(min(MIB,limit-count+1)),b''):
            count+=len(data);require(count<=limit,'growing raw source exceeds bound');h.update(data)
    return h.hexdigest(),count

def archive(source,destination,raw_cap=192*MIB,compressed_cap=48*MIB):
    identity=raw_identity(source);require(identity[2]<=raw_cap, 'raw artifact exceeds cap')
    expected,count=bounded_hash(source,identity[2]);require(count==identity[2],'source size changed')
    # Caller holds the producer lease. Both hashes and compression are streaming bounded.
    consumed=0
    with source.open('rb') as inp, destination.open('xb') as out:
        with gzip.GzipFile(fileobj=BoundedWriter(out,compressed_cap),mode='wb',filename='',mtime=0) as compressed:
            while True:
                chunk=inp.read(min(MIB,identity[2]-consumed+1))
                if not chunk:break
                consumed+=len(chunk);require(consumed<=identity[2] and consumed<=raw_cap,'growing source during compression')
                compressed.write(chunk)
        out.flush();os.fsync(out.fileno())
    actual,count=bounded_hash(source,identity[2])
    require(raw_identity(source)==identity and consumed==count==identity[2] and actual==expected, 'source changed during compression')
    observed=readback(destination,expected,identity[2],raw_cap)
    return {'source':str(source),'raw_sha256':expected,'raw_size':identity[2],
        'archive':str(destination),'archive_sha256':digest(destination),
        'archive_size':destination.stat().st_size,'complete_readback':observed}

def readback(path,expected,size,cap=192*MIB):
    identity=raw_identity(path);h=hashlib.sha256();count=0
    with gzip.open(path,'rb') as stream:
        for chunk in iter(lambda:stream.read(MIB),b''):
            count+=len(chunk);require(count<=cap, 'decompressed bound exceeded');h.update(chunk)
    require(raw_identity(path)==identity, 'archive changed during readback')
    require(count==size and h.hexdigest()==expected,'complete gzip readback differs')
    return {'raw_sha256':h.hexdigest(),'raw_size':count,'eof_checked':True}
