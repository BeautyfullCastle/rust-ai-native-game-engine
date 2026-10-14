"""Focused Linux HUD command owner; no privileged or persistent setup.

WNOWAIT protects the leader's group until the last group signal. A process-local
child subreaper catches orphaned descendants, including separate sessions; pidfds
and kernel child-wait ownership fence each subsequent signal/reap. ECHILD is the
terminal ownership predicate. No process-name search or adoption of unrelated
processes is used. This is process lifecycle control, not a memory/cgroup limit.
"""
import ctypes,errno,json,os,pathlib,resource,selectors,signal,subprocess,time
MIB=1024**2

class OwnershipUnknown(RuntimeError):pass

def require(value,message):
    if not value:raise RuntimeError(message)

def kernel_no_children():
    try:os.waitid(os.P_ALL,0,os.WEXITED|os.WNOHANG|os.WNOWAIT)
    except ChildProcessError:return True
    return False

def setup():
    require(signal.getsignal(signal.SIGCHLD)==signal.SIG_DFL,'SIGCHLD must retain wait ownership')
    require(all(hasattr(os,name) for name in ['WNOWAIT','P_PIDFD','pidfd_open']) and hasattr(signal,'pidfd_send_signal'),'Linux pidfd/wait ownership required')
    require(kernel_no_children(),'owner starts with unrelated children')
    libc=ctypes.CDLL(None,use_errno=True)
    require(libc.prctl(36,1,0,0,0)==0,'PR_SET_CHILD_SUBREAPER unavailable')
    value=ctypes.c_int();require(libc.prctl(37,ctypes.byref(value),0,0,0)==0 and value.value==1,'subreaper setting not verified')
    return {'subreaper':True,'pidfd_available':True,'initial_kernel_ECHILD':True,'aggregate_memory_limit_created':False}

def adopted_ids():
    path=pathlib.Path('/proc/self/task')/str(os.getpid())/'children'
    text=path.read_text();require(len(text)<=65536,'owned child inventory cap')
    ids=[int(x) for x in text.split()];require(len(ids)<=4096 and len(ids)==len(set(ids)),'owned child count cap')
    return ids

def reap_descendants(deadline,records):
    """Called only after the retained leader's final group signal and reap."""
    while time.monotonic()<deadline:
        if kernel_no_children():return
        ids=adopted_ids()
        for pid in ids:
            fd=os.pidfd_open(pid)
            try:
                # This syscall rejects a non-child, even if a PID were recycled.
                state=os.waitid(os.P_PIDFD,fd,os.WEXITED|os.WNOHANG|os.WNOWAIT)
                if state is None:
                    signal.pidfd_send_signal(fd,signal.SIGKILL)
                    records.append({'adopted_pid':pid,'signal':'SIGKILL','identity':'pidfd plus kernel child wait'})
                state=os.waitid(os.P_PIDFD,fd,os.WEXITED|os.WNOHANG|os.WNOWAIT)
                if state is not None:
                    result=os.waitid(os.P_PIDFD,fd,os.WEXITED)
                    records.append({'reaped_pid':result.si_pid,'code':result.si_code,'status':result.si_status})
            except (ChildProcessError,ProcessLookupError) as error:
                raise OwnershipUnknown('lost pidfd child ownership') from error
            finally:os.close(fd)
        time.sleep(.02)
    raise OwnershipUnknown('descendants remain after bounded exact-owned cleanup')

class Runner:
    def __init__(self,evidence,env,observe,total_timeout=10500):
        self.evidence=evidence;self.env=env;self.observe=observe
        self.deadline=time.monotonic()+total_timeout;self.pending=[];self.unknown=False
        self.old={s:signal.getsignal(s) for s in [signal.SIGINT,signal.SIGTERM]}
        for s in self.old:signal.signal(s,lambda signum,frame:self.pending.append(signum))
        self.capability=setup()
    def run(self,name,argv,cwd,expected=0,timeout=3600,extra_env=None):
        require(not self.unknown and not self.pending,'interrupted/UNKNOWN owner cannot launch')
        require(kernel_no_children(),'previous owned descendants remain before next command')
        require(all(c.isalnum() or c in '-_' for c in name),'unsafe command label')
        deadline=min(self.deadline,time.monotonic()+timeout);require(time.monotonic()<deadline,'total gate deadline')
        process=None;leader_reaped=False;clean=False;reason=None;records=[];status=None
        selector=selectors.DefaultSelector();started=time.time_ns();counts={'stdout':0,'stderr':0};next_sample=0
        streams={key:(self.evidence/(name+'.'+key)).open('xb') for key in counts}
        sample=(self.evidence/(name+'.samples.jsonl')).open('xb')
        try:
            self.observe()
            # Latch before launch. It clears only after a kernel ECHILD proof.
            self.unknown=True
            process=subprocess.Popen(argv,cwd=cwd,env=self.env if extra_env is None else extra_env,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,start_new_session=True)
            for key,pipe in [('stdout',process.stdout),('stderr',process.stderr)]:
                os.set_blocking(pipe.fileno(),False);selector.register(pipe,selectors.EVENT_READ,key)
            while True:
                now=time.monotonic()
                if self.pending:reason=reason or 'controller signal'
                if now>=deadline:reason=reason or 'command/whole-gate deadline'
                try:observation=self.observe()
                except BaseException as error:reason=reason or 'resource observation: '+repr(error);observation={'error':repr(error)}
                if now>=next_sample:
                    data=(json.dumps({'ns':str(time.time_ns()),'observation':observation,'child_maxrss_kib_observed':resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,'aggregate_memory_hard_limit':False})+'\n').encode()
                    require(sample.tell()+len(data)<=4*MIB,'command samples cap');sample.write(data);next_sample=now+2
                for key,_ in selector.select(.1):
                    data=os.read(key.fileobj.fileno(),65536)
                    if not data:selector.unregister(key.fileobj);key.fileobj.close();continue
                    if counts[key.data]+len(data)>32*MIB:
                        reason=reason or 'command stream cap';data=data[:max(0,32*MIB-counts[key.data])]
                    streams[key.data].write(data);counts[key.data]+=len(data)
                state=os.waitid(os.P_PID,process.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
                if state is not None or reason:break
            # Preserve the unreaped leader while signaling the exact owned group.
            os.waitid(os.P_PID,process.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
            try:os.killpg(process.pid,signal.SIGTERM)
            except ProcessLookupError:pass
            grace=time.monotonic()+2
            while time.monotonic()<grace:time.sleep(.02)
            os.waitid(os.P_PID,process.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
            try:os.killpg(process.pid,signal.SIGKILL)
            except ProcessLookupError:pass
            # Wait is bounded, and does not signal a reaped/recycled group.
            try:status=process.wait(timeout=10);leader_reaped=True
            except subprocess.TimeoutExpired as error:raise OwnershipUnknown('leader did not exit after KILL') from error
            reap_descendants(time.monotonic()+15,records)
            require(kernel_no_children(),'terminal ECHILD missing')
            clean=True;self.unknown=False
            # Descendant cleanup must also close every inherited output FD.
            drain=time.monotonic()+2
            while selector.get_map() and time.monotonic()<drain:
                for key,_ in selector.select(.05):
                    data=os.read(key.fileobj.fileno(),65536)
                    if not data:selector.unregister(key.fileobj);key.fileobj.close();continue
                    if counts[key.data]+len(data)>32*MIB:reason=reason or 'stream cap during drain';data=data[:max(0,32*MIB-counts[key.data])]
                    streams[key.data].write(data);counts[key.data]+=len(data)
            require(not selector.get_map(),'owned descendants left output descriptors open')
        except BaseException as error:
            reason=reason or repr(error)
            # An exception before normal cleanup is never promoted to success.
            if process is not None and not leader_reaped:
                try:
                    os.waitid(os.P_PID,process.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
                    try:os.killpg(process.pid,signal.SIGKILL)
                    except ProcessLookupError:pass
                    status=process.wait(timeout=10);leader_reaped=True
                    reap_descendants(time.monotonic()+15,records)
                    clean=kernel_no_children();self.unknown=not clean
                except BaseException:clean=False;self.unknown=True
            raise
        finally:
            selector.close()
            for stream in [*streams.values(),sample]:stream.flush();os.fsync(stream.fileno());stream.close()
            for pipe in ([] if process is None else [process.stdout,process.stderr]):
                if not pipe.closed:pipe.close()
            receipt={'argv':list(argv),'cwd':str(cwd),'started_ns':str(started),'finished_ns':str(time.time_ns()),'actual_exit':status,'expected_exit':expected,'stop_reason':reason,'kernel_ECHILD':clean,'ownership_unknown':self.unknown,'descendant_cleanup':records,'stream_bytes':counts,'aggregate_memory_hard_limit':False}
            with (self.evidence/(name+'.transport.json')).open('x') as stream:json.dump(receipt,stream,indent=2)
        require(clean and not reason and status==expected,'owned command failed: '+name)
        return (self.evidence/(name+'.stdout')).read_text(),(self.evidence/(name+'.stderr')).read_text()
