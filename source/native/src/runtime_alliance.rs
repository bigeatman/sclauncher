//! Own-player diplomacy. Explicit launcher checkbox requests authorize the ordinary
//! own-player alliance sender. Original native Confirm/Cancel are not command triggers.
use super::*;
use crate::{alliance, alliance_dialog as dialog, alliance_requests as requests};
use std::io::Read;
use std::path::PathBuf;
use std::time::SystemTime;
use winapi::um::synchapi::SetEvent;

pub(super) struct Config {
    pub game:Value, pub players:Value, pub policy:Option<Policy>,
}
pub(super) struct Policy {
    pub game_data:Value,
    pub matcher_count:Value, pub matcher_string:Value,
}
impl Config {
    fn snapshot(&self,owner:u8)->Option<alliance::Snapshot> {
        alliance::read_snapshot(self.game.read()?,self.players.read()?,owner,read_memory)
    }
    fn allowed(&self)->bool {
        self.policy.as_ref().is_some_and(Policy::allowed)
    }
}
impl Policy {
    fn allowed(&self)->bool {
        (||Some(read_integer(self.game_data.read()?.checked_add(0x7c)?,1)? == 1
            && read_integer(self.game_data.read()?.checked_add(0x7f)?,1)? == 0
            && (self.matcher_count.read()? as u32 as i32) <= 0
            && read_integer(self.matcher_string.read()?.checked_add(8)?,8)? == 0))() == Some(true)
    }
}
struct Diplomacy {
    session:u64,generation:u64,frame:u32,in_game:bool,owner:u8,session_boundary:u64,
    identity:Option<[usize;3]>,
    target:Option<dialog::Discovery>, snapshot:Option<alliance::Snapshot>,
    staged:Option<requests::Edits>, last_request:u64,ack:u64,allowed:bool,
    area:[i32;4],canvas:[i32;2],message:&'static str,pending_apply:Option<PendingApply>,confirmation:Option<ConfirmationScope>,
}
static DIPLOMACY:Mutex<Diplomacy> = Mutex::new(Diplomacy {
    session:0,generation:0,frame:0,in_game:false,owner:255,session_boundary:0,identity:None,target:None,snapshot:None,
    staged:None,last_request:0,ack:0,allowed:false,area:[0;4],canvas:[0;2],message:"Waiting for alliance dialog",pending_apply:None,confirmation:None,
});
static ORIGINAL:AtomicUsize = AtomicUsize::new(0);
static DISPATCHING:AtomicBool = AtomicBool::new(false);
// Reentrant alliance dispatch or dispatch on another thread invalidates the
// outer observation even when the outgoing prefix happens to remain equal.
static REENTRY_EPOCH:AtomicU64 = AtomicU64::new(0);
static DATA_FOLDER:OnceLock<PathBuf> = OnceLock::new();
static REQUEST_EPOCH:OnceLock<SystemTime> = OnceLock::new();
struct Dispatch;
impl Drop for Dispatch {fn drop(&mut self){DISPATCHING.store(false,Ordering::Release);}}

fn discovery_checked(runtime:&Runtime)->Result<Option<dialog::Discovery>,&'static str> {
    let gui=runtime.gui.as_ref().ok_or("Alliance UI operands unavailable")?;
    let first=gui.first_dialog.read().ok_or("Alliance UI state unavailable")?;
    let found=dialog::discover(first,callback as *const () as usize,
        |p|p>=gui.code_start&&p.checked_add(16).is_some_and(|end|end<=gui.code_end),read_memory)
        .map_err(|_|"Alliance dialog layout unavailable")?;
    if gui.first_dialog.read()!=Some(first){return Err("Alliance UI state changed; retrying");}
    Ok(found)
}
fn discovery(runtime:&Runtime)->Option<dialog::Discovery> {
    discovery_checked(runtime).ok().flatten()
}
/// Uses the actual console extents, including widescreen. Zoomed world viewport
/// dimensions are deliberately not used as GUI dimensions.
fn canvas_from_metadata(text:&str)->Option<[i32;2]> {
    let mut width=0;let mut height=0;let mut names=HashSet::new();
    for line in text.lines() {
        let fields:Vec<_>=line.split('\t').collect();
        if fields.len()!=10||fields[0]!="ROOT"||fields[3]!="1" {continue;}
        if !matches!(fields[9],"Minimap"|"StatBtn"|"StatRes"|"StatPort") {continue;}
        let right=fields[7].parse::<i32>().ok()?;let bottom=fields[8].parse::<i32>().ok()?;
        width=width.max(right.checked_add(1)?);height=height.max(bottom.checked_add(1)?);
        names.insert(fields[9]);
    }
    if !names.contains("Minimap")||!names.contains("StatBtn")||!names.contains("StatRes")
        ||!(640..=4096).contains(&width)||height!=480 {return None;}
    Some([width,height])
}
fn canvas(runtime:&Runtime)->Option<[i32;2]> {
    let gui=runtime.gui.as_ref()?;let first=gui.first_dialog.read()?;
    let meta=crate::alliance_ui_probe::inspect(first,read_memory)?;
    let result=canvas_from_metadata(&meta.text)?;
    (gui.first_dialog.read()==Some(first)).then_some(result)
}
fn row_area(found:&dialog::Discovery,count:usize)->Option<[i32;4]> {
    if count==0||count>found.frame.available_rows.len(){return None;}
    let rows=&found.frame.available_rows[..count];let first=rows.first()?;let last=rows.last()?;
    let x=i32::from(found.frame.area.left);let y=i32::from(found.frame.area.top);
    Some([x+i32::from(first.label.left),y+i32::from(first.label.top)-2,
        x+i32::from(first.alliance.right)+6,y+i32::from(last.label.bottom)+6])
}
fn request_path(pid:u32)->Option<PathBuf> {Some(DATA_FOLDER.get()?.join(format!("mc-{pid}-alliance-apply.tsv")))}
fn request_timestamp_valid(modified:SystemTime,epoch:SystemTime,now:SystemTime)->bool {
    now>=epoch&&modified>=epoch&&modified<=now
}
fn request_timestamp_fresh(modified:SystemTime,epoch:SystemTime,now:SystemTime)->bool {
    request_timestamp_valid(modified,epoch,now)
        &&now.duration_since(modified).is_ok_and(|age|age<=Duration::from_secs(3))
}
fn context_continues(before:Option<(bool,u32,u8)>,after:Option<(bool,u32,u8)>)->bool {
    matches!((before,after),(Some((true,old_frame,owner)),Some((true,new_frame,next_owner)))
        if owner<8&&owner==next_owner&&new_frame>=old_frame)
}
fn read_current_request(path:&Path,epoch:SystemTime)->Option<requests::Edits> {
    let mut file=fs::File::open(path).ok()?;
    let before=file.metadata().ok()?;let modified=before.modified().ok()?;
    if before.len()>requests::MAX_REQUEST_BYTES as u64
        ||!request_timestamp_fresh(modified,epoch,SystemTime::now()){return None;}
    let mut bytes=Vec::with_capacity(before.len() as usize);
    // The metadata length cannot authorize an unbounded allocation/read if a
    // writer changes the file. Read from this opened file, with a hard bound.
    (&mut file).take(requests::MAX_REQUEST_BYTES as u64+1).read_to_end(&mut bytes).ok()?;
    let after=file.metadata().ok()?;
    if after.len()!=before.len()||bytes.len() as u64!=before.len()
        ||after.modified().ok()?!=modified
        ||!request_timestamp_fresh(modified,epoch,SystemTime::now()){return None;}
    requests::parse_apply(&bytes)
}
fn consume_edits(d:&mut Diplomacy,pid:u32) {
    if !d.allowed||d.target.is_none(){d.staged=None;return;}
    let parsed=(|| {
        let path=request_path(pid)?;
        read_current_request(&path,*REQUEST_EPOCH.get()?)
    })();
    let Some(edits)=parsed else{return;};
    if edits.pid!=pid||edits.session!=d.session||edits.generation!=d.generation
        ||edits.frame>d.frame||edits.request_id<=d.last_request {return;}
    d.last_request=edits.request_id;
    if d.snapshot.as_ref().is_none_or(|s|!requests::validate(&edits,s)) {
        d.staged=None;d.message="Alliance edits stale; reopen dialog";return;
    }
    d.staged=(!edits.changes.is_empty()).then_some(edits);
    d.message=if d.staged.is_some(){"Alliance edits staged; use native Confirm"}else{"Alliance dialog ready"};
}
// Remember explicit checkbox intent only for this exact open native dialog.
// No periodic command is generated from this state. It is used solely to
// replace the computer bits in an ordinary ALLIANCE emitted by the original UI.
#[derive(Clone)]
struct ConfirmationScope {
    session:u64,identity:[usize;3],owner:u8,dialog:dialog::Discovery,
    players:[alliance::Player;8],desired:[Option<u8>;8],last:requests::Edits,consumed:bool,capture_sources:u8,
}
static OUTPUT_CAPTURES:AtomicUsize = AtomicUsize::new(0);
static OUTPUT_ROOT:AtomicUsize = AtomicUsize::new(0);
static OUTPUT_INTERFERENCE:AtomicU64 = AtomicU64::new(0);
static OUTPUT_HISTORY:Mutex<(crate::alliance_output_diagnostics::History,bool)> = Mutex::new((crate::alliance_output_diagnostics::History::new(),false));
struct OutputLease {owned:bool}
impl OutputLease {
    fn try_enter()->Option<Self> {
        OUTPUT_CAPTURES.compare_exchange(0,1,Ordering::AcqRel,Ordering::Acquire).ok().map(|_|Self{owned:true})
    }
}
impl Drop for OutputLease {
    fn drop(&mut self){if self.owned{OUTPUT_ROOT.store(0,Ordering::Release);OUTPUT_CAPTURES.store(0,Ordering::Release);}}
}
/// Same-thread descendants belong to the existing transaction. A different
/// thread/root or a launcher-owned append invalidates that transaction only.
pub(super) fn note_callback(control:Option<usize>) {
    if OUTPUT_CAPTURES.load(Ordering::Acquire)==0{return;}
    let thread=unsafe{GetCurrentThreadId()};let root=OUTPUT_ROOT.load(Ordering::Acquire);
    if thread!=EVENT_THREAD.load(Ordering::Acquire)||control.is_some_and(|address|root!=0&&address!=root) {
        OUTPUT_INTERFERENCE.fetch_add(1,Ordering::AcqRel);
    }
}
pub(super) fn note_owned_append() {
    if OUTPUT_CAPTURES.load(Ordering::Acquire)!=0 {OUTPUT_INTERFERENCE.fetch_add(1,Ordering::AcqRel);}
}
fn scope_same_dialog(scope:&ConfirmationScope,d:&Diplomacy,found:&dialog::Discovery,snapshot:&alliance::Snapshot)->bool {
    d.in_game&&d.session==scope.session&&d.identity==Some(scope.identity)
        &&d.owner==scope.owner&&snapshot.owner==scope.owner&&snapshot.players==scope.players
        &&scope.dialog==*found&&snapshot.has_valid_core()
}
fn remember_confirmation(d:&mut Diplomacy,edits:&requests::Edits,found:&dialog::Discovery,snapshot:&alliance::Snapshot) {
    let Some(identity)=d.identity else{return;};
    if !d.in_game||edits.session!=d.session||snapshot.owner!=d.owner||!snapshot.has_valid_core(){return;}
    let same=d.confirmation.as_ref().is_some_and(|scope|scope_same_dialog(scope,d,found,snapshot));
    if !same {d.confirmation=Some(ConfirmationScope {
        session:d.session,identity,owner:d.owner,dialog:found.clone(),players:snapshot.players,
        desired:[None;8],last:edits.clone(),consumed:false,capture_sources:0,
    });}
    let scope=d.confirmation.as_mut().unwrap();
    for change in &edits.changes {
        if snapshot.computer(change.slot).is_some()&&change.desired<=1 {
            scope.desired[change.slot as usize]=Some(change.desired);
        }
    }
    scope.last=edits.clone();scope.capture_sources=0;
}
fn pending_expected(d:&Diplomacy,slot:u8,current:u8)->bool {
    d.pending_apply.as_ref().is_some_and(|pending|pending.edits.session==d.session
        &&unsafe{GetTickCount64()}.saturating_sub(pending.submitted_tick)<5000
        &&pending.edits.changes.iter().any(|edit|edit.slot==slot&&edit.expected==current))
}
fn scope_values(d:&Diplomacy,scope:&ConfirmationScope,snapshot:&alliance::Snapshot)->[Option<u8>;8] {
    let mut desired=scope.desired;
    for slot in 0..8 {
        let Some(value)=desired[slot]else{continue;};
        let current=snapshot.alliance_row[slot];
        let matches=if value==1{matches!(current,1|2)}else{current==0};
        if snapshot.computer(slot as u8).is_none()||(!matches&&!pending_expected(d,slot as u8,current)) {
            desired[slot]=None;
        }
    }
    desired
}
fn refresh_confirmation(d:&mut Diplomacy) {
    let Some(scope)=d.confirmation.as_ref()else{return;};
    let Some(found)=d.target.as_ref()else{d.confirmation=None;return;};
    let Some(snapshot)=d.snapshot.as_ref()else{d.confirmation=None;return;};
    if !scope_same_dialog(scope,d,found,snapshot) {d.confirmation=None;return;}
    let desired=scope_values(d,scope,snapshot);
    if desired.iter().all(Option::is_none){d.confirmation=None;}
    else{d.confirmation.as_mut().unwrap().desired=desired;}
}
pub(super) struct OutputCapture {
    buffer:BufferSnapshot,auth:crate::alliance_output_context::Authorization,binding:u64,reentry:u64,
    scope:ConfirmationScope,snapshot:alliance::Snapshot,desired:[Option<u8>;8],source:u8,session_boundary:u64,_lease:OutputLease,
}
fn output_observation(runtime:&Runtime)->crate::alliance_output_context::Observation {
    let now=unsafe{GetTickCount64()};
    crate::alliance_output_context::Observation {
        ui_thread:unsafe{GetCurrentThreadId()},context:runtime.context(),identity:session_identity(runtime),
        now_tick:now,interference:OUTPUT_INTERFERENCE.load(Ordering::Acquire),
        installed:INSTALLED.load(Ordering::Acquire),fault:FAULT.load(Ordering::Acquire),
        foreground:foreground_is_game(),heartbeat:client_present(now,CLIENT_SEEN_TICK.load(Ordering::Acquire)),
        in_original:IN_ORIGINAL.with(Cell::get),paused_clear:runtime.paused.read()==Some(0),
        policy:runtime.alliance.as_ref().is_some_and(Config::allowed),
        stop_clear:stop_is_clear(STOP_EVENT.load(Ordering::Acquire)),
    }
}
pub(super) fn begin_output(control:Option<usize>)->Option<OutputCapture> {
    if !session_allows_control(){return None;}
    // A descendant wrapper forwards the same native call; only the outermost
    // complete original call observes/replaces its native output.
    let interference=OUTPUT_INTERFERENCE.load(Ordering::Acquire);
    let start_tick=unsafe{GetTickCount64()};
    let lease=OutputLease::try_enter()?;
    let runtime=RUNTIME.get()?;let observation=output_observation(runtime);
    let context=observation.context.filter(|c|c.0)?;let identity=observation.identity?;
    let auth=crate::alliance_output_context::Authorization {
        ui_thread:EVENT_THREAD.load(Ordering::Acquire),before:context,identity,
        start_tick,interference,
    };
    if crate::alliance_output_context::rejection_mask(&auth,&observation)!=0{return None;}
    let found=discovery(runtime)?;
    if control.is_some_and(|address|address!=found.target.control){return None;}
    OUTPUT_ROOT.store(found.target.control,Ordering::Release);
    let snapshot=runtime.alliance.as_ref()?.snapshot(context.2)?;
    let mut d=DIPLOMACY.lock().ok()?;let scope=d.confirmation.as_ref()?;
    if !session_is_current(d.session)||scope.consumed||!scope_same_dialog(scope,&d,&found,&snapshot)||scope.identity!=identity
        ||scope.owner!=context.2||context.1<d.frame||!d.allowed{return None;}
    let desired=scope_values(&d,scope,&snapshot);
    if desired.iter().all(Option::is_none){return None;}
    let source=if control.is_some(){1}else{2};
    if scope.capture_sources&source==0 {
        let last=scope.last.clone();d.confirmation.as_mut().unwrap().capture_sources|=source;
        log_apply(&last,context.1,if source==1{ApplyCode::CaptureRoot}else{ApplyCode::CaptureOuter},0,0);
    }
    let scope=d.confirmation.as_ref().unwrap().clone();drop(d);
    let buffer=buffer_snapshot(runtime)?;
    if discovery(runtime).as_ref()!=Some(&found)
        ||crate::alliance_output_context::rejection_mask(&auth,&output_observation(runtime))!=0{return None;}
    Some(OutputCapture{buffer,auth,binding:BINDING_EPOCH.load(Ordering::Acquire),
        reentry:REENTRY_EPOCH.load(Ordering::Acquire),scope,snapshot,desired,source,
        session_boundary:SESSION_BOUNDARY_EPOCH.load(Ordering::Acquire),_lease:lease})
}
fn output_reason(c:&OutputCapture,runtime:&Runtime)->u32 {
    let current=session_allows_control();
    crate::alliance_output_context::rejection_mask(&c.auth,&output_observation(runtime))
        |if !current||!session_is_current(c.scope.session)
            ||c.session_boundary!=SESSION_BOUNDARY_EPOCH.load(Ordering::Acquire) {OUTPUT_REJECT_SCOPE}else{0}
}
fn record_output(c:&OutputCapture,runtime:&Runtime,mask:u32) {
    let record=crate::alliance_output_diagnostics::Record {
        request_id:c.scope.last.request_id,session:c.scope.session,generation:c.scope.last.generation,
        source:c.source,before_frame:c.auth.before.1,after_frame:runtime.context().map(|c|c.1).unwrap_or(0),mask,
        nested:CALLBACK_NESTED.with(Cell::get),reentry_changed:REENTRY_EPOCH.load(Ordering::Acquire)!=c.reentry,
        binding_changed:BINDING_EPOCH.load(Ordering::Acquire)!=c.binding,
    };
    let mut history=OUTPUT_HISTORY.lock().unwrap_or_else(|p|p.into_inner());
    if history.0.observe(record){history.1=true;}
}
const OUTPUT_REJECT_SNAPSHOT:u32=1<<16;
const OUTPUT_REJECT_SCOPE:u32=1<<17;
const OUTPUT_REJECT_TARGET:u32=1<<18;
const OUTPUT_REJECT_BUFFER:u32=1<<19;
const OUTPUT_REJECT_DELTA:u32=1<<20;
const OUTPUT_REJECT_DIALOG:u32=1<<21;
const OUTPUT_REJECT_WRITE:u32=1<<22;
fn output_failure(c:&OutputCapture,runtime:&Runtime,mask:u32,code:ApplyCode,before:usize,after:usize) {
    record_output(c,runtime,mask);
    // Detailed context reasons are deduplicated in their bounded numeric log.
    // Do not flood the application history for normal no-output UI ticks.
    log_apply(&c.scope.last,runtime.context().map(|c|c.1).unwrap_or(c.auth.before.1),code,before,after);
}
fn scope_still_authorized(d:&Diplomacy,c:&OutputCapture)->bool {
    d.in_game&&d.session==c.scope.session&&d.owner==c.scope.owner&&d.identity==Some(c.auth.identity)
        &&d.confirmation.as_ref().is_some_and(|scope|!scope.consumed&&scope.last.request_id==c.scope.last.request_id
            &&scope.session==c.scope.session&&scope.dialog==c.scope.dialog&&scope.players==c.scope.players
            &&scope.desired==c.scope.desired)
}
fn current_output_values(d:&Diplomacy,c:&OutputCapture,now:&alliance::Snapshot)->Option<[Option<u8>;8]> {
    if !scope_still_authorized(d,c)||!now.has_valid_core()||now.owner!=c.auth.before.2||now.players!=c.snapshot.players{return None;}
    let mut desired=scope_values(d,&c.scope,now);
    // Never add a newly authorized slot after starting this native call.
    for slot in 0..8 {if desired[slot]!=c.desired[slot]{desired[slot]=None;}}
    desired.iter().any(Option::is_some).then_some(desired)
}
fn consume_output(d:&mut Diplomacy,c:&OutputCapture) {
    if scope_still_authorized(d,c){d.confirmation.as_mut().unwrap().consumed=true;}
}
pub(super) fn finish_output(c:OutputCapture) {
    let Some(runtime)=RUNTIME.get()else{return;};
    let _sender=SEND_LOCK.lock().unwrap_or_else(|p|p.into_inner());
    let mask=output_reason(&c,runtime);
    if mask!=0 {output_failure(&c,runtime,mask,ApplyCode::OutputContext,0,0);return;}
    let Some(after)=buffer_snapshot(runtime)else {
        output_failure(&c,runtime,OUTPUT_REJECT_BUFFER,ApplyCode::OutputBuffer,c.buffer.bytes.len(),0);return;
    };
    if after.buffer!=c.buffer.buffer||after.capacity!=c.buffer.capacity {
        output_failure(&c,runtime,OUTPUT_REJECT_BUFFER,ApplyCode::OutputBuffer,c.buffer.bytes.len(),after.bytes.len());return;
    }
    let Some(offset)=requests::native_alliance_offset(&c.buffer.bytes,&after.bytes)else {
        if c.buffer.bytes!=after.bytes||discovery(runtime).is_none(){
            output_failure(&c,runtime,OUTPUT_REJECT_DELTA,ApplyCode::OutputDelta,c.buffer.bytes.len(),after.bytes.len());
        }
        return;
    };
    let Some(original):Option<[u8;5]>=after.bytes.get(offset..offset+5).and_then(|bytes|bytes.try_into().ok())else{return;};
    log_apply(&c.scope.last,runtime.context().map(|c|c.1).unwrap_or(c.auth.before.1),ApplyCode::NativeDelta,c.buffer.bytes.len(),after.bytes.len());
    let Some(now)=runtime.alliance.as_ref().and_then(|config|config.snapshot(c.auth.before.2))else {
        output_failure(&c,runtime,OUTPUT_REJECT_SNAPSHOT,ApplyCode::ConfirmRejected,c.buffer.bytes.len(),after.bytes.len());return;
    };
    let desired={
        let d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());
        current_output_values(&d,&c,&now)
    };
    let Some(desired)=desired else {
        output_failure(&c,runtime,OUTPUT_REJECT_SCOPE,ApplyCode::ConfirmRejected,c.buffer.bytes.len(),after.bytes.len());return;
    };
    let Some(packet)=requests::merge_authorized_native(&original,&now,&desired)else {
        output_failure(&c,runtime,OUTPUT_REJECT_TARGET,ApplyCode::ConfirmRejected,c.buffer.bytes.len(),after.bytes.len());return;
    };
    // Closing the original dialog may free it. Rediscover only from the current
    // root list. Accept disappearance or the same current verified object.
    match discovery_checked(runtime) {
        Ok(None)=>(),Ok(Some(found)) if found==c.scope.dialog=>(),_=> {
            output_failure(&c,runtime,OUTPUT_REJECT_DIALOG,ApplyCode::ConfirmRejected,c.buffer.bytes.len(),after.bytes.len());return;
        }
    }
    let mask=output_reason(&c,runtime);
    if mask!=0 {output_failure(&c,runtime,mask,ApplyCode::OutputContext,c.buffer.bytes.len(),after.bytes.len());return;}
    let Some(fresh_snapshot)=runtime.alliance.as_ref().and_then(|config|config.snapshot(c.auth.before.2))else{return;};
    {
        let d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());
        if current_output_values(&d,&c,&fresh_snapshot)!=Some(desired) {
            drop(d);output_failure(&c,runtime,OUTPUT_REJECT_SCOPE,ApplyCode::ConfirmRejected,c.buffer.bytes.len(),after.bytes.len());return;
        }
    }
    if !buffer_snapshot(runtime).as_ref().is_some_and(|fresh|fresh.buffer==after.buffer
        &&fresh.capacity==after.capacity&&fresh.bytes==after.bytes) {
        output_failure(&c,runtime,OUTPUT_REJECT_BUFFER,ApplyCode::OutputBuffer,c.buffer.bytes.len(),after.bytes.len());return;
    }
    if original==packet {
        let mut d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());consume_output(&mut d,&c);
        drop(d);record_output(&c,runtime,0);
        log_apply(&c.scope.last,runtime.context().map(|c|c.1).unwrap_or(c.auth.before.1),ApplyCode::ConfirmUnchanged,c.buffer.bytes.len(),after.bytes.len());return;
    }
    let Some(address)=after.buffer.checked_add(offset)else{return;};
    // Replace the original native five bytes. No corrective append is emitted.
    let written=unsafe{crate::buffer_rewrite::rewrite_exact_5(address,&original,&packet)};
    let mut expected=after.bytes.clone();expected[offset..offset+5].copy_from_slice(&packet);
    let verified=written&&buffer_snapshot(runtime).as_ref().is_some_and(|fresh|
        fresh.buffer==after.buffer&&fresh.capacity==after.capacity&&fresh.bytes==expected);
    let mut d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());
    if scope_still_authorized(&d,&c) {
        // Even uncertain writes are not retried as if nothing was written.
        consume_output(&mut d,&c);
        d.message=if verified{"Native computer alliance values preserved"}else{"Native alliance rewrite not confirmed"};
    }
    drop(d);record_output(&c,runtime,if verified{0}else{OUTPUT_REJECT_WRITE});
    log_apply(&c.scope.last,runtime.context().map(|c|c.1).unwrap_or(c.auth.before.1),
        if verified{ApplyCode::ConfirmRewritten}else{ApplyCode::ConfirmWriteUnconfirmed},c.buffer.bytes.len(),after.bytes.len());
}
fn confirmation_should_expire(scope:&ConfirmationScope,context:Option<(bool,u32,u8)>,identity:Option<[usize;3]>,found:Option<&dialog::Discovery>)->bool {
    scope.consumed||context.is_none_or(|c|!c.0||c.2!=scope.owner)
        ||identity!=Some(scope.identity)||found!=Some(&scope.dialog)
}
pub(super) fn end_output_callback() {
    if OUTPUT_CAPTURES.load(Ordering::Acquire)!=0{return;}
    let Some(runtime)=RUNTIME.get()else{return;};
    let found=discovery(runtime);let context=runtime.context();let identity=session_identity(runtime);
    let mut d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());
    if d.confirmation.as_ref().is_some_and(|scope|confirmation_should_expire(scope,context,identity,found.as_ref())) {
        let scope=d.confirmation.take().unwrap();
        log_apply(&scope.last,context.map(|c|c.1).unwrap_or(d.frame),ApplyCode::ScopeExpired,0,0);
    }
}
struct PendingApply {edits:requests::Edits,submitted_tick:u64}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
enum ApplyCode {Received,Context,Policy,State,Buffer,Stopped,Busy,Appended,AlreadyApplied,AppendUnconfirmed,Applied,TimedOut,ConfirmRewritten,ConfirmUnchanged,ConfirmRejected,ConfirmWriteUnconfirmed,CaptureRoot,CaptureOuter,NativeDelta,OutputContext,OutputBuffer,OutputDelta,ScopeExpired}
impl ApplyCode {
    fn code(self)->&'static str {match self {
        Self::Received=>"RECEIVED",Self::Context=>"REJECTED_CONTEXT",Self::Policy=>"REJECTED_POLICY",
        Self::State=>"REJECTED_STATE",Self::Buffer=>"REJECTED_BUFFER",Self::Stopped=>"REJECTED_STOPPED",Self::Busy=>"REJECTED_BUSY",
        Self::Appended=>"APPENDED",Self::AlreadyApplied=>"ALREADY_APPLIED",
        Self::AppendUnconfirmed=>"APPEND_UNCONFIRMED",Self::Applied=>"APPLIED",Self::TimedOut=>"TIMED_OUT",
        Self::ConfirmRewritten=>"CONFIRM_REWRITTEN",Self::ConfirmUnchanged=>"CONFIRM_UNCHANGED",
        Self::ConfirmRejected=>"CONFIRM_REJECTED",Self::ConfirmWriteUnconfirmed=>"CONFIRM_WRITE_UNCONFIRMED",
        Self::CaptureRoot=>"ROOT_CAPTURED",Self::CaptureOuter=>"OUTER_CAPTURED",Self::NativeDelta=>"NATIVE_DELTA_ACCEPTED",
        Self::OutputContext=>"OUTPUT_REJECTED_CONTEXT",Self::OutputBuffer=>"OUTPUT_REJECTED_BUFFER",
        Self::OutputDelta=>"OUTPUT_NO_NATIVE_DELTA",Self::ScopeExpired=>"SCOPE_CLOSED",
    }}
}
#[derive(Clone,Copy,Debug,Eq,PartialEq)]
struct ApplyRecord {session:u64,generation:u64,frame:u32,request_id:u64,code:ApplyCode,before_len:u16,after_len:u16}
struct ApplyHistory {records:VecDeque<ApplyRecord>,dirty:bool}
impl ApplyHistory {
    const fn new()->Self {Self{records:VecDeque::new(),dirty:false}}
    fn push(&mut self,record:ApplyRecord) {
        if self.records.back()==Some(&record){return;}
        if self.records.len()==64{self.records.pop_front();}
        self.records.push_back(record);self.dirty=true;
    }
    fn render(&self,pid:u32)->String {
        let mut text=format!("SCALLYAPPLYLOG1\t{pid}\t{}\n",self.records.len());
        for r in &self.records {text.push_str(&format!("A\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            r.code.code(),r.session,r.generation,r.frame,r.request_id,r.before_len,r.after_len));}
        text
    }
}
static APPLY_HISTORY:Mutex<ApplyHistory> = Mutex::new(ApplyHistory::new());
fn log_apply(edits:&requests::Edits,frame:u32,code:ApplyCode,before:usize,after:usize) {
    APPLY_HISTORY.lock().unwrap_or_else(|p|p.into_inner()).push(ApplyRecord {
        session:edits.session,generation:edits.generation,frame,request_id:edits.request_id,code,
        before_len:before.min(512) as u16,after_len:after.min(512) as u16,
    });
}
fn apply_context_matches(d:&Diplomacy,edits:&requests::Edits,pid:u32,context:(bool,u32,u8),identity:[usize;3],found:&dialog::Discovery)->bool {
    context.0&&d.in_game&&d.allowed&&d.owner==context.2&&context.1>=d.frame
        &&d.identity==Some(identity)&&d.target.as_ref()==Some(found)
        &&edits.pid==pid&&edits.session==d.session&&edits.generation==d.generation
        &&edits.frame<=context.1&&edits.request_id>d.last_request&&!edits.changes.is_empty()
}
fn desired_relations_match(edits:&requests::Edits,snapshot:&alliance::Snapshot)->bool {
    snapshot.has_valid_core()&&!edits.changes.is_empty()&&edits.changes.iter().all(|edit|
        snapshot.computer(edit.slot).is_some()&&snapshot.alliance_row[edit.slot as usize]==edit.desired)
}
fn observe_applied(d:&mut Diplomacy) {
    let Some(pending)=d.pending_apply.take()else{return;};
    let edits=&pending.edits;
    // A sent command remains in-flight after the dialog closes/reopens.
    if !d.in_game||d.session!=edits.session {
        log_apply(edits,d.frame,ApplyCode::Context,0,0);return;
    }
    if d.snapshot.as_ref().is_some_and(|s|desired_relations_match(edits,s)) {
        if edits.generation==d.generation&&d.ack<edits.request_id&&d.confirmation.as_ref().is_none_or(|scope|!scope.consumed) {
            if let (Some(found),Some(snapshot))=(d.target.clone(),d.snapshot.clone()) {
                remember_confirmation(d,edits,&found,&snapshot);
            }
        }
        d.ack=d.ack.max(edits.request_id);
        d.message="Computer alliance applied";log_apply(edits,d.frame,ApplyCode::Applied,0,0);
    } else if unsafe{GetTickCount64()}.saturating_sub(pending.submitted_tick)>=5000 {
        d.message="Computer alliance not observed; check map rules";
        log_apply(edits,d.frame,ApplyCode::TimedOut,0,0);
    } else {d.pending_apply=Some(pending);}
}
/// UI pump holds SEND_LOCK; STATE is not yet locked. Only explicit launcher
/// checkbox requests authorize this path. Native closure and old staging files
/// are not application requests.
pub(super) fn apply_requested(runtime:&Runtime) {
    if EVENT_THREAD.load(Ordering::Acquire)!=unsafe{GetCurrentThreadId()}
        ||!ready_to_capture()||DISPATCHING.load(Ordering::Acquire) {return;}
    let pid=unsafe{GetCurrentProcessId()};
    let Some(path)=request_path(pid)else{return;};let Some(epoch)=REQUEST_EPOCH.get()else{return;};
    let Some(edits)=read_current_request(&path,*epoch)else{return;};

    if DISPATCHING.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err(){return;}
    let _dispatch=Dispatch;
    let Some(context)=runtime.context().filter(|c|c.0)else{return;};
    let Some(identity)=session_identity(runtime)else{return;};
    let Some(found)=discovery(runtime)else{return;};
    let mut d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());
    if edits.pid!=pid||edits.request_id<=d.last_request{return;}
    log_apply(&edits,context.1,ApplyCode::Received,0,0);
    let valid_context=session_is_current(edits.session)&&apply_context_matches(&d,&edits,pid,context,identity,&found);
    d.last_request=d.last_request.max(edits.request_id);
    if !valid_context {d.message="Computer alliance request expired";log_apply(&edits,context.1,ApplyCode::Context,0,0);return;}
    if d.pending_apply.is_some() {d.message="Computer alliance previous request pending";log_apply(&edits,context.1,ApplyCode::Busy,0,0);return;}
    if runtime.paused.read()!=Some(0)||!runtime.alliance.as_ref().is_some_and(Config::allowed) {
        d.message="Alliance changes not allowed";log_apply(&edits,context.1,ApplyCode::Policy,0,0);return;
    }
    let Some(snapshot)=runtime.alliance.as_ref().and_then(|c|c.snapshot(context.2))else{
        log_apply(&edits,context.1,ApplyCode::State,0,0);return;
    };
    if desired_relations_match(&edits,&snapshot) {
        remember_confirmation(&mut d,&edits,&found,&snapshot);
        d.ack=edits.request_id;d.message="Computer alliance applied";
        log_apply(&edits,context.1,ApplyCode::AlreadyApplied,0,0);return;
    }
    let Some(packet)=requests::apply_to_current(&edits,&snapshot)else{
        d.message="Computer alliance state changed; retry";log_apply(&edits,context.1,ApplyCode::State,0,0);return;
    };
    let Some(before)=buffer_snapshot(runtime)else{log_apply(&edits,context.1,ApplyCode::Buffer,0,0);return;};
    if before.bytes.len().checked_add(packet.len()).is_none_or(|n|n>OUTGOING_BUDGET.min(before.capacity)) {
        d.message="Computer alliance sender busy; retry";log_apply(&edits,context.1,ApplyCode::Buffer,before.bytes.len(),0);return;
    }
    let Some(gui)=runtime.gui.as_ref()else{return;};let Some(first)=gui.first_dialog.read()else{return;};
    if dialog::recheck(first,&found,callback as *const () as usize,
        |p|p>=gui.code_start&&p.checked_add(16).is_some_and(|end|end<=gui.code_end),read_memory).is_err()
        ||gui.first_dialog.read()!=Some(first)||runtime.context()!=Some(context)
        ||session_identity(runtime)!=Some(identity)||runtime.alliance.as_ref().and_then(|c|c.snapshot(context.2)).as_ref()!=Some(&snapshot) {
        log_apply(&edits,context.1,ApplyCode::State,before.bytes.len(),0);return;
    }
    let binding=BINDING_EPOCH.load(Ordering::Acquire);
    drop(d); // Stop helper can acquire DIPLOMACY, so never hold it here.
    if !ready_to_capture()||BINDING_EPOCH.load(Ordering::Acquire)!=binding
        ||runtime.context()!=Some(context)||session_identity(runtime)!=Some(identity)
        ||runtime.paused.read()!=Some(0)||!runtime.alliance.as_ref().is_some_and(Config::allowed)
        ||runtime.alliance.as_ref().and_then(|c|c.snapshot(context.2)).as_ref()!=Some(&snapshot)
        ||discovery(runtime).as_ref()!=Some(&found) {
        log_apply(&edits,context.1,ApplyCode::Stopped,before.bytes.len(),0);return;
    }
    if !buffer_snapshot(runtime).as_ref().is_some_and(|fresh|fresh.buffer==before.buffer
        &&fresh.capacity==before.capacity&&fresh.bytes==before.bytes) {
        log_apply(&edits,context.1,ApplyCode::Buffer,before.bytes.len(),0);return;
    }
    unsafe{forward_original(packet.as_ptr(),packet.len());}
    let after=buffer_snapshot(runtime);
    let appended=after.as_ref().is_some_and(|a|a.buffer==before.buffer&&a.capacity==before.capacity
        &&a.bytes.len()==before.bytes.len()+packet.len()&&a.bytes.starts_with(&before.bytes)
        &&a.bytes[before.bytes.len()..]==packet);
    let mut d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());
    // Once the sender was called, absence of an observed append is not proof
    // that nothing was sent. Keep the fence until actual state or timeout.
    d.pending_apply=Some(PendingApply{edits:edits.clone(),submitted_tick:unsafe{GetTickCount64()}});
    if appended {
        remember_confirmation(&mut d,&edits,&found,&snapshot);
        d.ack=edits.request_id;d.message="Computer alliance sent; awaiting actual relation";
        log_apply(&edits,context.1,ApplyCode::Appended,before.bytes.len(),after.as_ref().map(|a|a.bytes.len()).unwrap_or(0));
    } else {
        d.message="Computer alliance sender append not confirmed";
        log_apply(&edits,context.1,ApplyCode::AppendUnconfirmed,before.bytes.len(),after.as_ref().map(|a|a.bytes.len()).unwrap_or(0));
    }
}
// Public SCR control ABI, pinned bw_dat/src/bw/structs.rs:914-923.
// Only show/hide/redraw ext events are emitted. Native click handlers stay intact.
#[repr(C)]
struct ControlEvent {
    ext_type:usize,ext_param:usize,param:u64,ty:u16,x:i16,y:i16,time:u32,
}
static BUTTON_STATE:AtomicU32 = AtomicU32::new(0);
static HISTORY:Mutex<(crate::alliance_diagnostics::History,bool)> = Mutex::new((crate::alliance_diagnostics::History::new(),false));
struct OriginalGuard(bool);
impl OriginalGuard {
    fn enter()->Self {Self(IN_ORIGINAL.with(|flag|flag.replace(true)))}
}
impl Drop for OriginalGuard {
    fn drop(&mut self){IN_ORIGINAL.with(|flag|flag.set(self.0));}
}
fn button_state(value:u32) {BUTTON_STATE.store(value,Ordering::Release);}
pub(super) fn maintain_button(runtime:&Runtime) {
    use crate::minimap_alliance_button as button;
    if !session_allows_control()||EVENT_THREAD.load(Ordering::Acquire)!=unsafe{GetCurrentThreadId()}
        || IN_ORIGINAL.with(Cell::get)||DISPATCHING.load(Ordering::Acquire)
        || !INSTALLED.load(Ordering::Acquire)||!CALLBACKS_READY.load(Ordering::Acquire)
        || FAULT.load(Ordering::Acquire)
        || !client_present(unsafe{GetTickCount64()},CLIENT_SEEN_TICK.load(Ordering::Acquire)) {
        button_state(0);return;
    }
    let Some(context)=runtime.context().filter(|c|c.0)else{button_state(0);return;};
    let Some(config)=runtime.alliance.as_ref()else{button_state(1);return;};
    let Some(snapshot)=config.snapshot(context.2)else{button_state(1);return;};
    let Some(identity)=session_identity(runtime)else{button_state(1);return;};
    let Some(gui)=runtime.gui.as_ref()else{button_state(1);return;};
    let Some(first)=gui.first_dialog.read()else{button_state(1);return;};
    let is_code=|p:usize|p>=gui.code_start&&p.checked_add(16).is_some_and(|end|end<=gui.code_end);
    let is_owned_root=|p:usize|owned_minimap_provider(p,MINIMAP_ORIGINAL.load(Ordering::Acquire),
        minimap_callback as *const () as usize,is_code);
    let Ok(Some(found))=button::discover_with_root_provider(first,is_code,is_owned_root,read_memory)else{button_state(1);return;};
    let desired=snapshot.has_alliance_targets();
    let Some(intent)=found.intent(desired)else{button_state(if found.alliance.visible==desired {2}else{5});return;};
    // Current UI-thread discovery is rechecked immediately. No cached heap
    // pointers survive between pumps, or between hide and its redraw event.
    if gui.first_dialog.read()!=Some(first)||runtime.context()!=Some(context)
        ||session_identity(runtime)!=Some(identity)||config.snapshot(context.2).as_ref()!=Some(&snapshot)
        ||button::recheck_with_root_provider(first,&found,is_code,is_owned_root,read_memory).is_err()
        ||gui.first_dialog.read()!=Some(first)
        ||!stop_is_clear(STOP_EVENT.load(Ordering::Acquire)) {
        button_state(5);return;
    }
    let mut event=ControlEvent {ext_type:intent.ext_type as usize,ext_param:0,param:0,
        ty:0xe,x:0,y:0,time:unsafe{winapi::um::sysinfoapi::GetTickCount()}};
    let original:PanelCallback=unsafe{std::mem::transmute(intent.callback)};
    let _guard=OriginalGuard::enter();
    let result=unsafe{original(intent.control as *const c_void,(&mut event as *mut ControlEvent).cast())};
    if intent.followup_ext_type(result).is_some() {
        // Redraw is separate; rediscover current object after the native hide.
        if gui.first_dialog.read()==Some(first) {
            if let Ok(Some(fresh))=button::discover_with_root_provider(first,is_code,is_owned_root,read_memory) {
                if fresh.minimap.control==found.minimap.control
                    &&fresh.minimap.callback==found.minimap.callback&&fresh.minimap.area==found.minimap.area
                    &&fresh.alliance.control==intent.control&&fresh.alliance.callback==intent.callback
                    &&fresh.alliance.id==found.alliance.id&&fresh.alliance.ty==found.alliance.ty
                    &&fresh.alliance.area==found.alliance.area&&!fresh.alliance.visible
                    &&runtime.context()==Some(context)&&session_identity(runtime)==Some(identity)
                    &&config.snapshot(context.2).as_ref()==Some(&snapshot)
                    &&button::recheck_with_root_provider(first,&fresh,is_code,is_owned_root,read_memory).is_ok()
                    &&gui.first_dialog.read()==Some(first) {
                    event.ext_type=button::EXT_HIDE_FOLLOWUP;
                    unsafe{original(intent.control as *const c_void,(&mut event as *mut ControlEvent).cast());}
                }
            }
        }
    }
    // Record observed state, not merely the fact that an event was sent.
    let fresh=gui.first_dialog.read().and_then(|current|button::discover_with_root_provider(current,is_code,is_owned_root,read_memory).ok().flatten());
    let applied=fresh.as_ref().is_some_and(|now|now.alliance.control==intent.control
        &&now.alliance.callback==intent.callback&&now.alliance.visible==snapshot.has_alliance_targets());
    button_state(if applied {if snapshot.has_alliance_targets(){3}else{4}}else{5});
}
pub(super) fn bind(runtime:&Runtime) {
    if !session_allows_control()||runtime.alliance.is_none()||EVENT_THREAD.load(Ordering::Acquire)!=unsafe{GetCurrentThreadId()}{return;}
    let Some(found)=discovery(runtime)else{return;};let replacement=callback as *const () as usize;
    if found.target.callback==replacement{return;}
    let old=ORIGINAL.load(Ordering::Acquire);
    if old!=0&&old!=found.target.callback{return;}
    let Some(gui)=runtime.gui.as_ref()else{return;};let Some(first)=gui.first_dialog.read()else{return;};
    if dialog::recheck(first,&found,replacement,
        |p|p>=gui.code_start&&p.checked_add(16).is_some_and(|end|end<=gui.code_end),read_memory).is_err(){return;}
    if ORIGINAL.compare_exchange(0,found.target.callback,Ordering::AcqRel,Ordering::Acquire)
        .is_err_and(|existing|existing!=found.target.callback){return;}
    let slot=crate::callback_binding::Slot {address:found.target.slot_address,original:found.target.callback,replacement};
    let _=unsafe{crate::callback_binding::install(&[slot])};
}
pub(super) fn publish(folder:&Path,pid:u32,ready:bool)->std::io::Result<()> {
    let _=DATA_FOLDER.set(folder.to_path_buf());
    let _=REQUEST_EPOCH.get_or_init(SystemTime::now);
    if DISPATCHING.load(Ordering::Acquire)||OUTPUT_CAPTURES.load(Ordering::Acquire)!=0{return Ok(());}
    let runtime=RUNTIME.get();let context=runtime.and_then(|r|r.context());
    let identity=runtime.and_then(session_identity);
    let live=ready&&context.is_some_and(|c|c.0)&&client_present(unsafe{GetTickCount64()},CLIENT_SEEN_TICK.load(Ordering::Acquire));
    let mut d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());
    if DISPATCHING.load(Ordering::Acquire)||OUTPUT_CAPTURES.load(Ordering::Acquire)!=0{return Ok(());}
    let frame=context.map(|c|c.1).unwrap_or(0);let owner=context.map(|c|c.2).unwrap_or(255);
    // Readiness/heartbeat loss is not a new game. One shared verified game
    // generation controls both unit and diplomacy lifetimes.
    sync_diplomacy_session(&mut d,GAME_SESSION_GENERATION.load(Ordering::Acquire),SESSION_BOUNDARY_EPOCH.load(Ordering::Acquire));
    d.in_game=live;d.frame=frame;d.owner=owner;d.identity=identity;
    let mut snapshot=if live {runtime.and_then(|r|r.alliance.as_ref()).and_then(|c|c.snapshot(owner))}else{None};
    let observed=if snapshot.is_some(){runtime.map(discovery_checked)}else{None};
    let mut found=observed.as_ref().and_then(|result|result.as_ref().ok()).cloned().flatten();
    if d.target!=found||d.snapshot.as_ref().zip(snapshot.as_ref()).is_some_and(|(old,new)|old.players!=new.players) {
        d.generation=d.generation.saturating_add(1);d.staged=None;
    }
    d.allowed=false;d.area=[0;4];d.canvas=[0;2];
    if let (Some(r),Some(found),Some(snapshot))=(runtime,found.as_ref(),snapshot.as_ref()) {
        let count=editable_computer_count(snapshot);
        if let (Some(area),Some(view))=(row_area(found,count),canvas(r)) {
            if area[0]>=0&&area[1]>=0&&area[2]<=view[0]&&area[3]<=view[1]
                &&area[3]<i32::from(found.frame.area.top)+i32::from(found.frame.allied_victory.top) {
                d.area=area;d.canvas=view;
                d.allowed=r.alliance.as_ref().is_some_and(Config::allowed)&&r.paused.read()==Some(0)
                    &&found.frame.confirm.enabled;
            }
        }
    }
    // UI worker reads are observations, not a game-thread lifetime lease. A
    // Advancing frames are normal on the worker thread. The same live owner,
    // monotonic frame, session identity, complete roster/outgoing row and root
    // must remain consistent; native output allows monotonic frame advancement.
    let observed_context=runtime.and_then(|r|r.context());
    let coherent=!live||runtime.is_some_and(|r|context_continues(context,observed_context)&&session_identity(r)==identity
        // A missing first roster is a data error, not a changed UI observation.
        // Compare root/roster only after an initial snapshot was obtained.
        &&snapshot.as_ref().is_none_or(|expected|
            r.alliance.as_ref().and_then(|config|config.snapshot(owner)).as_ref()==Some(expected)
            &&discovery(r).as_ref()==found.as_ref())
        &&context_continues(observed_context,r.context()));
    if !coherent {
        snapshot=None;found=None;d.allowed=false;d.area=[0;4];d.canvas=[0;2];d.staged=None;
        d.generation=d.generation.saturating_add(1);
    }
    d.message=if !live {"Waiting for game state"}
        else if runtime.is_none_or(|r|r.alliance.is_none()) {"Alliance runtime operands unavailable"}
        else if !coherent {"Alliance state changed; retrying"}
        else if snapshot.is_none() {"Alliance player state unavailable"}
        else if let Some(Err(message))=observed.as_ref() {*message}
        else if found.is_none() {
            if terminal_diagnostic(d.message){d.message}else{"Waiting for alliance dialog"}
        }
        else if snapshot.as_ref().is_some_and(|s|editable_computer_count(s)==0) {"No editable computer players"}
        else if d.area==[0;4] {"Alliance dialog layout unavailable"}
        else if runtime.and_then(|r|r.alliance.as_ref()).is_some_and(|c|c.policy.is_none()) {"Alliance policy unavailable; display only"}
        else if !d.allowed {"Alliance changes not allowed"}
        else if d.staged.is_some() {"Alliance edits staged; use native Confirm"}
        else if terminal_diagnostic(d.message) {d.message}
        else {"Alliance dialog ready"};
    d.target=found;d.snapshot=snapshot;refresh_confirmation(&mut d);observe_applied(&mut d);
    let open=d.target.is_some()&&d.area!=[0;4];
    let mut text=format!("SCALLY1\t{pid}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        d.session,d.generation,d.frame,u8::from(open),u8::from(d.allowed),d.area[0],d.area[1],d.area[2],d.area[3],d.canvas[0],d.canvas[1],d.ack);
    if let Some(snapshot)=d.snapshot.as_ref() {
        for player in &snapshot.players {
            if editable_computer(snapshot,player.slot) {
                text.push_str(&format!("C\t{}\t16\t{}\n",player.slot,snapshot.alliance_row[player.slot as usize]));
            }
        }
    }
    use crate::alliance_diagnostics::{Phase,Record};
    let phase=if !live {Phase::Waiting}
        else if runtime.is_none_or(|r|r.alliance.is_none()){Phase::CoreUnavailable}
        else if !coherent{Phase::StateChanged}
        else if d.snapshot.is_none(){Phase::PlayersUnavailable}
        else if observed.as_ref().is_some_and(Result::is_err){Phase::DialogUnavailable}
        else if d.target.is_none(){Phase::DialogClosed}
        else if d.snapshot.as_ref().is_some_and(|s|editable_computer_count(s)==0){Phase::NoComputers}
        else if d.area==[0;4]{Phase::LayoutUnavailable}
        else if runtime.and_then(|r|r.alliance.as_ref()).is_some_and(|c|c.policy.is_none()){Phase::PolicyUnavailable}
        else if !d.allowed{Phase::PolicyBlocked}else{Phase::Ready};
    let record=Record {phase,session:d.session,frame:d.frame,owner:d.owner,
        other_humans:d.snapshot.as_ref().map(|s|s.other_human_count() as u8).unwrap_or(0),
        computers:d.snapshot.as_ref().map(|s|s.active_computer_count() as u8).unwrap_or(0),
        policy_available:runtime.and_then(|r|r.alliance.as_ref()).is_some_and(|c|c.policy.is_some()),
        button_state:if live{BUTTON_STATE.load(Ordering::Acquire) as u8}else{0}};
    let message=d.message;drop(d);
    // Preserve the observation even if either state publication fails.
    let mut history=HISTORY.lock().unwrap_or_else(|p|p.into_inner());
    if history.0.observe(record){history.1=true;}
    atomic_write_text(&folder.join(format!("mc-{pid}-alliance.tsv")),&text)?;
    atomic_write_text(&folder.join(format!("mc-{pid}-alliance-status.txt")),message)?;
    if history.1 {
        let rendered=history.0.render(pid);
        debug_assert!(rendered.len()<crate::alliance_diagnostics::MAX_RENDER_BYTES);
        atomic_write_text(&folder.join(format!("mc-{pid}-alliance-transitions.tsv")),&rendered)?;
        history.1=false;
    }
    drop(history);
    let mut output_history=OUTPUT_HISTORY.lock().unwrap_or_else(|p|p.into_inner());
    if output_history.1 {
        let rendered=output_history.0.render(pid);debug_assert!(rendered.len()<8192);
        atomic_write_text(&folder.join(format!("mc-{pid}-alliance-output-log.tsv")),&rendered)?;
        output_history.1=false;
    }
    drop(output_history);
    let mut apply_history=APPLY_HISTORY.lock().unwrap_or_else(|p|p.into_inner());
    if apply_history.dirty {
        let rendered=apply_history.render(pid);debug_assert!(rendered.len()<8192);
        atomic_write_text(&folder.join(format!("mc-{pid}-alliance-apply-log.tsv")),&rendered)?;
        apply_history.dirty=false;
    }
    Ok(())
}
fn editable_computer(snapshot:&alliance::Snapshot,slot:u8)->bool {
    snapshot.computer(slot).is_some()&&snapshot.alliance_row[slot as usize]<=2
}
fn editable_computer_count(snapshot:&alliance::Snapshot)->usize {
    snapshot.players.iter().filter(|player|editable_computer(snapshot,player.slot)).count()
}
fn sync_diplomacy_session(d:&mut Diplomacy,session:u64,boundary:u64) {
    if d.session==session&&d.session_boundary==boundary{return;}
    d.session=session;d.session_boundary=boundary;d.generation=d.generation.saturating_add(1);
    d.staged=None;d.pending_apply=None;d.confirmation=None;d.target=None;d.snapshot=None;
    d.ack=0;d.allowed=false;d.in_game=false;d.frame=0;d.owner=255;d.identity=None;
    d.area=[0;4];d.canvas=[0;2];d.message="Game session changed; waiting for current alliance dialog";
    // last_request is process-monotonic: an old file can never regain authority.
}
struct Capture {buffer:BufferSnapshot,context:(bool,u32,u8),session:[usize;3],generation:u64,
    binding_epoch:u64,reentry_epoch:u64,edits:requests::Edits,snapshot:alliance::Snapshot}
fn clear_staged_on_stop(d:&mut Diplomacy)->bool {
    let Some(edits)=d.staged.take()else{return false;};
    // The same on-disk request must not be staged again after the pump consumes
    // this stop event. A new UI generation also clears the displayed draft.
    d.last_request=d.last_request.max(edits.request_id);
    d.generation=d.generation.saturating_add(1);
    d.message="Alliance edits cancelled by stop request";
    true
}
fn cancel_unsubmitted_apply(d:&mut Diplomacy,request_id:Option<u64>) {
    d.confirmation=None;
    if let Some(id)=request_id {d.last_request=d.last_request.max(id);}
    if !clear_staged_on_stop(d) {d.generation=d.generation.saturating_add(1);}
    // A command already sent is not undone. Only unsubmitted intent expires.
    d.message="Computer alliance unsent requests cancelled";
}
pub(super) fn cancel_pending_on_stop() {
    let current=(|| {
        let request=read_current_request(&request_path(unsafe{GetCurrentProcessId()})?,*REQUEST_EPOCH.get()?)?;
        (request.pid==unsafe{GetCurrentProcessId()}).then_some(request.request_id)
    })();
    let mut d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());
    cancel_unsubmitted_apply(&mut d,current);
}
fn stop_wait_allows_capture(wait_result:u32,cancel:impl FnOnce(),restore:impl FnOnce()->bool)->bool {
    match wait_result {
        0 => {
            cancel();
            // STOP_EVENT is auto-reset. Our peek consumed it: return the signal
            // to the existing unit pump, even though this capture is rejected.
            let _=restore();
            false
        },
        0x102 => true,
        _ => false,
    }
}
fn stop_is_clear(stop:usize)->bool {
    stop==0||stop_wait_allows_capture(unsafe{WaitForSingleObject(stop as *mut c_void,0)},
        cancel_pending_on_stop,||unsafe{SetEvent(stop as *mut c_void)}!=0)
}
fn ready_to_capture()->bool {
    let stop=STOP_EVENT.load(Ordering::Acquire);
    INSTALLED.load(Ordering::Acquire)&&CALLBACKS_READY.load(Ordering::Acquire)
        &&session_allows_control()
        &&!FAULT.load(Ordering::Acquire)&&ORIGINAL_SEND.load(Ordering::Acquire)!=0
        &&!IN_ORIGINAL.with(Cell::get)
        &&foreground_is_game()
        &&client_present(unsafe{GetTickCount64()},CLIENT_SEEN_TICK.load(Ordering::Acquire))
        // This guard is called before any DIPLOMACY lock is acquired.
        &&stop_is_clear(stop)
}
fn staged_context_matches(d:&Diplomacy,context:(bool,u32,u8),identity:[usize;3],found:&dialog::Discovery)->bool {
    context.0&&d.in_game&&d.allowed&&d.owner==context.2&&context.1>=d.frame
        &&d.identity==Some(identity)&&d.target.as_ref()==Some(found)
}
fn snapshot_stable(before:&alliance::Snapshot,now:&alliance::Snapshot,edits:&requests::Edits)->bool {
    // Controller masks alone cannot distinguish a changed roster or a trigger
    // changing relationships during the original callback. Preserve the full
    // observed outgoing row, including slots outside the editable eight.
    before.owner==now.owner&&before.players==now.players&&before.alliance_row==now.alliance_row
        &&requests::validate(edits,now)
}
fn capture_epoch_matches(c:&Capture,binding:u64,reentry:u64,nested:bool)->bool {
    !nested&&c.binding_epoch==binding&&c.reentry_epoch==reentry
}
fn request_still_staged(d:&Diplomacy,c:&Capture)->bool {
    d.in_game&&d.allowed&&d.generation==c.generation&&d.session==c.edits.session
        &&d.owner==c.context.2&&d.identity==Some(c.session)&&d.staged.as_ref()==Some(&c.edits)
}
fn terminal_diagnostic(message:&str)->bool {
    matches!(message,"Alliance command appended; awaiting actual game state"
        |"Native alliance command contains edits; awaiting actual game state"
        |"Alliance sender append not confirmed"
        |"Alliance dialog closed; staged edits not confirmed"
        |"Alliance edits cancelled by stop request"
        |"Computer alliance unsent requests cancelled"
        |"Computer alliance applied"
        |"Computer alliance not observed; check map rules"
        |"Computer alliance sender append not confirmed"
        |"Computer alliance state changed; retry"
        |"Computer alliance request expired"
        |"Computer alliance previous request pending"
        |"Computer alliance sender busy; retry"
        |"Computer alliance sent; awaiting actual relation"
        |"Native computer alliance values preserved"
        |"Native alliance rewrite not confirmed")
}
fn invalidate_changed_dialog(d:&mut Diplomacy,dispatched:usize,fresh:Option<&dialog::Discovery>)->bool {
    let Some(stored)=d.target.as_ref()else{return false;};
    if stored.target.control!=dispatched||fresh==Some(stored){return false;}
    let unconfirmed=d.staged.is_some();
    d.target=None;d.staged=None;d.allowed=false;d.area=[0;4];d.canvas=[0;2];
    d.generation=d.generation.saturating_add(1);
    d.message=if terminal_diagnostic(d.message){d.message}
        else if unconfirmed{"Alliance dialog closed; staged edits not confirmed"}
        else{"Alliance dialog closed or changed"};
    true
}
fn observe_dialog_after_original(dispatched:usize) {
    // Discover from the current root list. Never dereference a saved heap root
    // after Confirm/Cancel may have freed it. An immediately reused address
    // with a new callback/layout is treated as a new dialog generation too.
    let fresh=RUNTIME.get().and_then(discovery);
    let mut d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());
    let _=invalidate_changed_dialog(&mut d,dispatched,fresh.as_ref());
}
fn begin(control:usize)->Option<Capture> {
    let runtime=RUNTIME.get()?;if !ready_to_capture()||CALLBACK_NESTED.with(Cell::get)
        ||runtime.paused.read()!=Some(0)||!runtime.alliance.as_ref().is_some_and(Config::allowed){return None;}
    let context=runtime.context()?;if !context.0{return None;}
    let session=session_identity(runtime)?;
    let found=discovery(runtime)?;if found.target.control!=control||found.target.callback!=callback as *const () as usize{return None;}
    let mut d=DIPLOMACY.lock().ok()?;
    if !staged_context_matches(&d,context,session,&found){return None;}
    let snapshot=runtime.alliance.as_ref()?.snapshot(context.2)?;
    d.snapshot=Some(snapshot.clone());
    d.frame=context.1;
    let edits=d.staged.clone()?;
    if edits.session!=d.session||edits.generation!=d.generation||edits.frame>context.1
        ||!requests::validate(&edits,&snapshot){return None;}
    let buffer=buffer_snapshot(runtime)?;
    if buffer.bytes.len().checked_add(16)?>OUTGOING_BUDGET.min(buffer.capacity){return None;}
    Some(Capture {buffer,context,session,generation:d.generation,
        binding_epoch:BINDING_EPOCH.load(Ordering::Acquire),reentry_epoch:REENTRY_EPOCH.load(Ordering::Acquire),
        edits,snapshot})
}
fn finish(c:Capture) {
    let Some(runtime)=RUNTIME.get()else{return;};
    let _send=SEND_LOCK.lock().unwrap_or_else(|p|p.into_inner());
    if !ready_to_capture()||!capture_epoch_matches(&c,BINDING_EPOCH.load(Ordering::Acquire),
        REENTRY_EPOCH.load(Ordering::Acquire),CALLBACK_NESTED.with(Cell::get))
        ||runtime.context()!=Some(c.context)||session_identity(runtime)!=Some(c.session)
        ||runtime.paused.read()!=Some(0)||!runtime.alliance.as_ref().is_some_and(Config::allowed){return;}
    let Some(after)=buffer_snapshot(runtime)else{return;};
    if after.buffer!=c.buffer.buffer||after.capacity!=c.buffer.capacity {
        log_apply(&c.edits,c.context.1,ApplyCode::OutputBuffer,c.buffer.bytes.len(),after.bytes.len());return;}
    let Some(packet)=requests::merge_native(&c.edits,&c.snapshot,&c.buffer.bytes,&after.bytes)else{return;};
    let Some(now)=runtime.alliance.as_ref().and_then(|a|a.snapshot(c.context.2))else{return;};
    if !snapshot_stable(&c.snapshot,&now,&c.edits){return;}
    let mut d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());
    if !request_still_staged(&d,&c){return;}
    // Native confirmation may already have encoded the exact requested values.
    // In that case confirm the existing append without creating a duplicate.
    let mut no_edits=c.edits.clone();no_edits.changes.clear();
    if requests::merge_native(&no_edits,&c.snapshot,&c.buffer.bytes,&after.bytes)==Some(packet) {
        d.ack=c.edits.request_id;d.staged=None;d.message="Native alliance command contains edits; awaiting actual game state";
        return;
    }
    if after.bytes.len().checked_add(packet.len()).is_none_or(|n|n>OUTGOING_BUDGET.min(after.capacity)){return;}
    // Do not hold the diplomacy mutex across any engine call. The sender lock
    // serializes owned appends, and DISPATCHING keeps the publisher away. The
    // engine sender's nested callbacks forward through IN_ORIGINAL.
    drop(d);
    unsafe {forward_original(packet.as_ptr(),packet.len());}
    let Some(appended)=buffer_snapshot(runtime)else{return;};
    let mut d=DIPLOMACY.lock().unwrap_or_else(|p|p.into_inner());
    if !request_still_staged(&d,&c){return;}
    if appended.buffer!=after.buffer||appended.capacity!=after.capacity
        ||appended.bytes.len()!=after.bytes.len()+5||!appended.bytes.starts_with(&after.bytes)
        ||appended.bytes[after.bytes.len()..]!=packet {d.message="Alliance sender append not confirmed";return;}
    d.ack=c.edits.request_id;d.staged=None;d.message="Alliance command appended; awaiting actual game state";
}
unsafe extern "C" fn callback(control:*const c_void,event:*const c_void)->u32 {
    let session=SessionCallback::enter();
    note_callback(Some(control as usize));
    let address=ORIGINAL.load(Ordering::Acquire);if address==0{return 0;}
    let original:PanelCallback=unsafe{std::mem::transmute(address)};
    if !session.active()||IN_ORIGINAL.with(Cell::get) {
        if session.wrong_thread(){FAULT.store(true,Ordering::Release);}
        if DISPATCHING.load(Ordering::Acquire){REENTRY_EPOCH.fetch_add(1,Ordering::AcqRel);}
        return unsafe{original(control,event)};
    }
    if DISPATCHING.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).is_err() {
        REENTRY_EPOCH.fetch_add(1,Ordering::AcqRel);
        return unsafe{original(control,event)};
    }
    let _dispatch=Dispatch;
    let old_depth=CALLBACK_DEPTH.with(|depth|{let old=depth.get();depth.set(old.saturating_add(1));old});
    let _depth=CallbackDepth(old_depth);
    let capture=std::panic::catch_unwind(||begin_output(Some(control as usize))).ok().flatten();
    // Exactly one original call, with the original two arguments and return.
    let result=unsafe{original(control,event)};
    if let Some(capture)=capture {let _=std::panic::catch_unwind(||finish_output(capture));}
    let _=std::panic::catch_unwind(||observe_dialog_after_original(control as usize));
    let _=std::panic::catch_unwind(end_output_callback);
    result
}

#[cfg(test)] mod tests {
    #[test] fn next_game_clears_old_checkbox_apply_scope_and_ack_but_never_reuses_request_id() {
        let mut d=owned_diplomacy();let edits=owned_edits();let found=owned_discovery();let snapshot=owned_snapshot();
        remember_confirmation(&mut d,&edits,&found,&snapshot);
        d.staged=Some(edits.clone());d.last_request=edits.request_id;d.ack=edits.request_id;
        d.pending_apply=Some(PendingApply{edits:edits.clone(),submitted_tick:0});
        sync_diplomacy_session(&mut d,4,2);
        assert_eq!(d.session,4);assert_eq!(d.session_boundary,2);
        assert_eq!(d.ack,0);assert_eq!(d.last_request,edits.request_id);
        assert!(d.confirmation.is_none()&&d.pending_apply.is_none()&&d.staged.is_none()&&d.target.is_none());
        assert!(!d.allowed&&!d.in_game);assert_eq!(d.generation,5);
        d.in_game=true;d.allowed=true;d.owner=0;d.identity=Some([0,1,1]);d.target=Some(found.clone());d.frame=1;
        assert!(!apply_context_matches(&d,&edits,edits.pid,(true,2,0),[0,1,1],&found));
    }
    #[test] fn temporary_readiness_loss_does_not_invent_a_new_game_generation() {
        let mut d=owned_diplomacy();let found=owned_discovery();let snapshot=owned_snapshot();
        remember_confirmation(&mut d,&owned_edits(),&found,&snapshot);
        let generation=d.generation;d.in_game=false;
        sync_diplomacy_session(&mut d,3,0);
        assert_eq!(d.session,3);assert_eq!(d.generation,generation);assert!(d.confirmation.is_some());
        d.in_game=true;sync_diplomacy_session(&mut d,3,0);
        assert_eq!(d.session,3);assert_eq!(d.generation,generation);
    }
    #[test] fn verified_game_end_revokes_scope_before_next_game_starts() {
        let mut d=owned_diplomacy();let found=owned_discovery();
        remember_confirmation(&mut d,&owned_edits(),&found,&owned_snapshot());
        sync_diplomacy_session(&mut d,3,1);
        assert_eq!(d.session,3);assert!(!d.in_game);assert!(d.confirmation.is_none());
        let generation=d.generation;sync_diplomacy_session(&mut d,3,1);assert_eq!(d.generation,generation);
        sync_diplomacy_session(&mut d,4,2);assert_eq!(d.session,4);assert!(d.generation>generation);
    }
    #[test] fn nested_native_root_close_preserves_outer_scope_until_output_is_rewritten() {
        let mut d=owned_diplomacy();let found=owned_discovery();let mut snapshot=owned_snapshot();snapshot.alliance_row[2]=1;
        remember_confirmation(&mut d,&owned_edits(),&found,&snapshot);let capture=owned_output(&d,snapshot.clone());
        assert!(invalidate_changed_dialog(&mut d,found.target.control,None));
        assert!(d.target.is_none());assert!(!d.allowed);assert!(d.confirmation.is_some());
        assert!(scope_still_authorized(&d,&capture));assert!(current_output_values(&d,&capture,&snapshot).is_some());
        consume_output(&mut d,&capture);assert!(d.confirmation.as_ref().unwrap().consumed);
    }
    #[test] fn conflicted_target_is_removed_without_reintroducing_untouched_computers() {
        let mut d=owned_diplomacy();let found=owned_discovery();let mut snapshot=owned_snapshot();
        snapshot.players[3].controller=alliance::COMPUTER;snapshot.alliance_row[2]=1;snapshot.alliance_row[3]=1;
        let mut edits=owned_edits();edits.changes.push(requests::Edit{slot:3,expected:0,desired:1});
        remember_confirmation(&mut d,&edits,&found,&snapshot);let capture=owned_output(&d,snapshot.clone());
        snapshot.alliance_row[3]=0;let desired=current_output_values(&d,&capture,&snapshot).unwrap();
        assert_eq!(desired[2],Some(1));assert_eq!(desired[3],None);
        let mut row=snapshot.alliance_row;row[2]=0;row[3]=0;
        let packet=requests::merge_authorized_native(&alliance::encode_row(&row).unwrap(),&snapshot,&desired).unwrap();
        row[2]=1;assert_eq!(packet,alliance::encode_row(&row).unwrap());
    }
    fn owned_output(d:&Diplomacy,snapshot:alliance::Snapshot)->OutputCapture {
        OutputCapture {
            buffer:BufferSnapshot{bytes:vec![0x0d,0x0e,0x7],buffer:0x30000,capacity:512},
            auth:crate::alliance_output_context::Authorization{ui_thread:17,before:(true,50,0),identity:[0,1,1],start_tick:1000,interference:4},
            binding:6,reentry:9,scope:d.confirmation.as_ref().unwrap().clone(),snapshot,
            desired:d.confirmation.as_ref().unwrap().desired,source:2,session_boundary:0,_lease:OutputLease{owned:false},
        }
    }
    #[test] fn whole_native_confirm_pipeline_keeps_checkbox_across_descendants_and_frame_advance() {
        let mut d=owned_diplomacy();let found=owned_discovery();let before=owned_snapshot();let edits=owned_edits();
        d.pending_apply=Some(PendingApply{edits:edits.clone(),submitted_tick:unsafe{GetTickCount64()}});
        remember_confirmation(&mut d,&edits,&found,&before);
        let capture=owned_output(&d,before.clone());
        // Native UI can finish a frame and dispatch synchronous descendants.
        // These are not unit-control copying authorization conditions.
        let observation=crate::alliance_output_context::Observation {
            ui_thread:17,context:Some((true,51,0)),identity:Some([0,1,1]),now_tick:1010,interference:4,
            installed:true,fault:false,foreground:true,heartbeat:true,in_original:false,
            paused_clear:true,policy:true,stop_clear:true,
        };
        assert_eq!(crate::alliance_output_context::rejection_mask(&capture.auth,&observation),0);
        let mut current=before.clone();current.alliance_row[2]=1;current.alliance_row[1]=0;
        let desired=current_output_values(&d,&capture,&current).unwrap();
        // Original UI sends stale computer bits and current unsaved human edits.
        let mut native_row=before.alliance_row;native_row[1]=2;native_row[2]=0;
        let mut native=alliance::encode_row(&native_row).unwrap();native[4]=0xab;
        let mut outgoing=capture.buffer.bytes.clone();outgoing.extend_from_slice(&native);outgoing.extend_from_slice(&[0x0d,7,0]);
        let length=outgoing.len();let original=outgoing.clone();
        let offset=requests::native_alliance_offset(&capture.buffer.bytes,&outgoing).unwrap();
        let packet=requests::merge_authorized_native(&outgoing[offset..offset+5],&current,&desired).unwrap();
        assert!(unsafe{crate::buffer_rewrite::rewrite_exact_5(outgoing.as_mut_ptr() as usize+offset,&native,&packet)});
        assert_eq!(outgoing.len(),length);assert_eq!(&outgoing[..offset],&original[..offset]);
        assert_eq!(&outgoing[offset+5..],&original[offset+5..]);
        native_row[2]=1;let mut expected=alliance::encode_row(&native_row).unwrap();expected[4]=0xab;
        assert_eq!(&outgoing[offset..offset+5],&expected);
        consume_output(&mut d,&capture);assert!(d.confirmation.as_ref().unwrap().consumed);
        assert!(confirmation_should_expire(d.confirmation.as_ref().unwrap(),Some((true,51,0)),Some([0,1,1]),None));
    }
    #[test] fn current_native_output_validation_rejects_trigger_and_changed_roster() {
        let mut d=owned_diplomacy();let found=owned_discovery();let mut snapshot=owned_snapshot();snapshot.alliance_row[2]=1;
        remember_confirmation(&mut d,&owned_edits(),&found,&snapshot);let capture=owned_output(&d,snapshot.clone());
        assert!(current_output_values(&d,&capture,&snapshot).is_some());
        snapshot.alliance_row[2]=0;assert!(current_output_values(&d,&capture,&snapshot).is_none());
        snapshot=capture.snapshot.clone();snapshot.players[2].controller=11;
        assert!(current_output_values(&d,&capture,&snapshot).is_none());
        snapshot=capture.snapshot.clone();snapshot.players[2].storm_id+=1;
        assert!(current_output_values(&d,&capture,&snapshot).is_none());
        snapshot=capture.snapshot.clone();d.session+=1;
        assert!(current_output_values(&d,&capture,&snapshot).is_none());
    }
    #[test] fn actual_checkbox_confirmation_can_progress_but_additional_output_slots_cannot_appear() {
        let mut d=owned_diplomacy();let found=owned_discovery();let mut snapshot=owned_snapshot();
        snapshot.players[3].controller=alliance::COMPUTER;
        let mut edits=owned_edits();edits.changes.push(requests::Edit{slot:3,expected:0,desired:1});
        d.pending_apply=Some(PendingApply{edits:edits.clone(),submitted_tick:unsafe{GetTickCount64()}});
        remember_confirmation(&mut d,&edits,&found,&snapshot);let mut capture=owned_output(&d,snapshot.clone());
        capture.desired[3]=None;
        snapshot.alliance_row[2]=1;snapshot.alliance_row[3]=1;
        let desired=current_output_values(&d,&capture,&snapshot).unwrap();assert_eq!(desired[2],Some(1));assert_eq!(desired[3],None);
    }
    #[test] fn one_outer_lease_prevents_nested_capture_without_discarding_parent() {
        assert_eq!(OUTPUT_CAPTURES.load(Ordering::Acquire),0);
        let parent=OutputLease::try_enter().unwrap();assert_eq!(OUTPUT_CAPTURES.load(Ordering::Acquire),1);
        assert!(OutputLease::try_enter().is_none());assert_eq!(OUTPUT_CAPTURES.load(Ordering::Acquire),1);
        drop(parent);assert_eq!(OUTPUT_CAPTURES.load(Ordering::Acquire),0);
        let next=OutputLease::try_enter().unwrap();drop(next);assert_eq!(OUTPUT_CAPTURES.load(Ordering::Acquire),0);
    }
    #[test] fn actual_unconfirmed_second_toggle_is_added_to_existing_same_dialog_scope() {
        let mut d=owned_diplomacy();let found=owned_discovery();let mut snapshot=owned_snapshot();
        snapshot.players[3].controller=alliance::COMPUTER;snapshot.alliance_row[2]=1;snapshot.alliance_row[3]=0;
        let first=owned_edits();remember_confirmation(&mut d,&first,&found,&snapshot);d.ack=first.request_id;
        let mut second=first.clone();second.request_id+=1;second.changes=vec![requests::Edit{slot:3,expected:0,desired:1}];
        d.pending_apply=Some(PendingApply{edits:second.clone(),submitted_tick:unsafe{GetTickCount64()}});
        snapshot.alliance_row[3]=1;d.snapshot=Some(snapshot);observe_applied(&mut d);
        let scope=d.confirmation.as_ref().unwrap();assert_eq!(scope.desired[2],Some(1));assert_eq!(scope.desired[3],Some(1));
        assert_eq!(scope.last.request_id,second.request_id);assert_eq!(d.ack,second.request_id);assert!(d.pending_apply.is_none());
    }
    #[test] fn consumed_native_output_expires_even_if_dialog_address_and_layout_are_reused() {
        let mut d=owned_diplomacy();let found=owned_discovery();let snapshot=owned_snapshot();
        remember_confirmation(&mut d,&owned_edits(),&found,&snapshot);
        let scope=d.confirmation.as_mut().unwrap();
        assert!(!confirmation_should_expire(scope,Some((true,50,0)),Some([0,1,1]),Some(&found)));
        scope.consumed=true;
        assert!(confirmation_should_expire(scope,Some((true,50,0)),Some([0,1,1]),Some(&found)));
        scope.consumed=false;
        assert!(confirmation_should_expire(scope,Some((true,50,0)),Some([0,1,1]),None));
        assert!(confirmation_should_expire(scope,Some((true,50,1)),Some([0,1,1]),Some(&found)));
    }
    #[test] fn confirm_scope_keeps_multiple_explicit_checkbox_values_in_native_packet() {
        let mut d=owned_diplomacy();let found=owned_discovery();let mut snapshot=owned_snapshot();
        let mut request=owned_edits();request.changes=vec![requests::Edit{slot:2,expected:0,desired:1}];
        snapshot.alliance_row[2]=1;d.snapshot=Some(snapshot.clone());
        remember_confirmation(&mut d,&request,&found,&snapshot);
        request.request_id+=1;request.changes=vec![requests::Edit{slot:3,expected:0,desired:1}];
        snapshot.players[3].controller=alliance::COMPUTER;snapshot.alliance_row[3]=1;
        // A changed roster creates a new lease; use that roster consistently.
        d.confirmation=None;remember_confirmation(&mut d,&owned_edits(),&found,&snapshot);
        remember_confirmation(&mut d,&request,&found,&snapshot);
        let desired=scope_values(&d,d.confirmation.as_ref().unwrap(),&snapshot);
        assert_eq!(desired[2],Some(1));assert_eq!(desired[3],Some(1));
        let mut row=snapshot.alliance_row;row[1]=2;row[2]=0;row[3]=0;row[11]=3;
        let mut original=alliance::encode_row(&row).unwrap();original[4]=0xab;
        let rewritten=requests::merge_authorized_native(&original,&snapshot,&desired).unwrap();
        let mut expected=row;expected[2]=1;expected[3]=1;
        let mut expected=alliance::encode_row(&expected).unwrap();expected[4]=0xab;
        assert_eq!(rewritten,expected);assert_eq!(rewritten.len(),original.len());
    }
    #[test] fn close_cannot_be_mistaken_for_permission_to_emit_a_second_command() {
        let mut d=owned_diplomacy();let found=owned_discovery();let mut snapshot=owned_snapshot();
        snapshot.alliance_row[2]=1;remember_confirmation(&mut d,&owned_edits(),&found,&snapshot);
        let scope=d.confirmation.as_ref().unwrap();
        assert!(requests::native_alliance_offset(&[0x0d,0,0],&[0x0d,0,0]).is_none());
        let before=vec![0x05,0x0e,0x0d];
        let mut after=before.clone();after.extend_from_slice(&[0x0d,1,0]);
        assert!(requests::native_alliance_offset(&before,&after).is_none());
        assert!(scope_same_dialog(scope,&d,&found,&snapshot));
        d.target=None;refresh_confirmation(&mut d);assert!(d.confirmation.is_none());
    }
    #[test] fn trigger_relation_change_expires_protection_without_periodic_reapply() {
        let mut d=owned_diplomacy();let found=owned_discovery();let mut snapshot=owned_snapshot();
        snapshot.alliance_row[2]=1;remember_confirmation(&mut d,&owned_edits(),&found,&snapshot);
        d.snapshot=Some(snapshot.clone());refresh_confirmation(&mut d);assert!(d.confirmation.is_some());
        snapshot.alliance_row[2]=0;d.snapshot=Some(snapshot);refresh_confirmation(&mut d);
        assert!(d.confirmation.is_none());assert!(d.pending_apply.is_none());
    }
    #[test] fn freshly_sent_unobserved_toggle_survives_fast_confirm_only_while_pending() {
        let mut d=owned_diplomacy();let found=owned_discovery();let snapshot=owned_snapshot();let edits=owned_edits();
        d.pending_apply=Some(PendingApply{edits:edits.clone(),submitted_tick:unsafe{GetTickCount64()}});
        remember_confirmation(&mut d,&edits,&found,&snapshot);
        assert_eq!(scope_values(&d,d.confirmation.as_ref().unwrap(),&snapshot)[2],Some(1));
        d.pending_apply=None;
        assert_eq!(scope_values(&d,d.confirmation.as_ref().unwrap(),&snapshot)[2],None);
    }
    #[test] fn scope_never_crosses_sessions_owners_rosters_or_reused_dialogs() {
        let mut d=owned_diplomacy();let found=owned_discovery();let mut snapshot=owned_snapshot();snapshot.alliance_row[2]=1;
        remember_confirmation(&mut d,&owned_edits(),&found,&snapshot);d.snapshot=Some(snapshot.clone());
        let scope=d.confirmation.as_ref().unwrap().clone();
        d.session+=1;assert!(!scope_same_dialog(&scope,&d,&found,&snapshot));d.session-=1;
        d.owner=1;assert!(!scope_same_dialog(&scope,&d,&found,&snapshot));d.owner=0;
        let mut other=found.clone();other.target.control+=8;
        assert!(!scope_same_dialog(&scope,&d,&other,&snapshot));
        other=found.clone();other.frame.confirm.control+=8;
        assert!(!scope_same_dialog(&scope,&d,&other,&snapshot));
        snapshot.players[2].storm_id+=1;assert!(!scope_same_dialog(&scope,&d,&found,&snapshot));
        d.in_game=false;refresh_confirmation(&mut d);assert!(d.confirmation.is_none());
    }
    #[test] fn stop_discards_confirmation_rewrite_without_undoing_sent_apply() {
        let mut d=owned_diplomacy();let found=owned_discovery();let snapshot=owned_snapshot();let edits=owned_edits();
        d.pending_apply=Some(PendingApply{edits:edits.clone(),submitted_tick:unsafe{GetTickCount64()}});
        remember_confirmation(&mut d,&edits,&found,&snapshot);assert!(d.confirmation.is_some());
        cancel_unsubmitted_apply(&mut d,None);assert!(d.confirmation.is_none());assert!(d.pending_apply.is_some());
    }
    #[test] fn disabled_computer_value_and_native_allied_victory_are_kept_distinct() {
        let mut d=owned_diplomacy();let found=owned_discovery();let mut snapshot=owned_snapshot();
        let mut edits=owned_edits();edits.changes[0]=requests::Edit{slot:2,expected:1,desired:0};
        snapshot.alliance_row[2]=0;remember_confirmation(&mut d,&edits,&found,&snapshot);
        let desired=scope_values(&d,d.confirmation.as_ref().unwrap(),&snapshot);assert_eq!(desired[2],Some(0));
        let mut row=snapshot.alliance_row;row[2]=2;row[1]=2;
        let packet=requests::merge_authorized_native(&alliance::encode_row(&row).unwrap(),&snapshot,&desired).unwrap();
        row[2]=0;assert_eq!(packet,alliance::encode_row(&row).unwrap());
    }
    use super::*;
    #[test] fn stop_expires_unsubmitted_file_without_undoing_an_already_sent_request() {
        let mut d=owned_diplomacy();let request=owned_edits();
        d.pending_apply=Some(PendingApply{edits:request,submitted_tick:unsafe{GetTickCount64()}});
        d.ack=8;let generation=d.generation;
        cancel_unsubmitted_apply(&mut d,Some(10));
        assert_eq!(d.last_request,10);assert!(d.generation>generation);
        assert_eq!(d.ack,8);assert!(d.pending_apply.is_some());assert!(d.staged.is_none());
        assert_eq!(d.message,"Computer alliance unsent requests cancelled");
        let old=d.generation;cancel_unsubmitted_apply(&mut d,None);
        assert!(d.generation>old);assert_eq!(d.last_request,10);assert_eq!(d.ack,8);
    }
    #[test] fn immediate_request_matches_ui_context_and_rejects_replay_or_closed_dialog() {
        let mut d=owned_diplomacy();let request=owned_edits();let found=owned_discovery();
        d.last_request=7;
        assert!(apply_context_matches(&d,&request,1,(true,50,0),[0,1,1],&found));
        for c in [(false,50,0),(true,49,0),(true,50,1)] {
            assert!(!apply_context_matches(&d,&request,1,c,[0,1,1],&found));
        }
        assert!(!apply_context_matches(&d,&request,2,(true,50,0),[0,1,1],&found));
        assert!(!apply_context_matches(&d,&request,1,(true,50,0),[0,1,2],&found));
        let mut other=request.clone();other.generation+=1;
        assert!(!apply_context_matches(&d,&other,1,(true,50,0),[0,1,1],&found));
        other=request.clone();other.frame=51;
        assert!(!apply_context_matches(&d,&other,1,(true,50,0),[0,1,1],&found));
        d.last_request=8;
        assert!(!apply_context_matches(&d,&request,1,(true,50,0),[0,1,1],&found));
        d.last_request=7;d.target=None;
        assert!(!apply_context_matches(&d,&request,1,(true,50,0),[0,1,1],&found));
    }
    #[test] fn actual_relation_confirmation_keeps_in_flight_after_dialog_closes_and_reopens() {
        let mut d=owned_diplomacy();let request=owned_edits();
        d.pending_apply=Some(PendingApply{edits:request.clone(),submitted_tick:unsafe{GetTickCount64()}});
        d.target=None;d.allowed=false;d.generation+=1;
        observe_applied(&mut d);
        assert!(d.pending_apply.is_some()); // close is not application or cancellation
        d.snapshot.as_mut().unwrap().alliance_row[2]=1;
        observe_applied(&mut d);
        assert!(d.pending_apply.is_none());assert_eq!(d.message,"Computer alliance applied");
        assert_eq!(d.ack,8); // Actual matching state confirms a submitted request.
    }
    #[test] fn actual_relation_confirmation_rejects_departed_target_and_wrong_session() {
        let request=owned_edits();let mut snapshot=owned_snapshot();snapshot.alliance_row[2]=1;
        assert!(desired_relations_match(&request,&snapshot));
        snapshot.players[2].controller=11;
        assert!(!desired_relations_match(&request,&snapshot));
        let mut d=owned_diplomacy();d.session+=1;
        d.pending_apply=Some(PendingApply{edits:request,submitted_tick:unsafe{GetTickCount64()}});
        observe_applied(&mut d);assert!(d.pending_apply.is_none());assert_eq!(d.ack,0);
    }
    #[test] fn direct_apply_file_is_fresh_on_the_opened_file_not_a_later_path_replacement() {
        let epoch=SystemTime::UNIX_EPOCH+Duration::from_secs(100);
        let now=epoch+Duration::from_secs(10);
        assert!(request_timestamp_fresh(now-Duration::from_secs(3),epoch,now));
        assert!(!request_timestamp_fresh(now-Duration::from_millis(3001),epoch,now));
        assert!(!request_timestamp_fresh(epoch-Duration::from_secs(1),epoch,now));
        assert!(!request_timestamp_fresh(now+Duration::from_secs(1),epoch,now));
    }
    #[test] fn application_log_preserves_append_and_actual_state_after_later_game_end() {
        let mut h=ApplyHistory::new();
        let record=|request_id,code|ApplyRecord{session:1,generation:2,frame:50,request_id,code,before_len:0,after_len:5};
        h.push(record(1,ApplyCode::Appended));h.push(record(1,ApplyCode::Applied));
        let text=h.render(701);
        assert!(text.contains("APPENDED\t1\t2\t50\t1"));assert!(text.contains("APPLIED\t1\t2\t50\t1"));
        for id in 2..=100{h.push(record(id,ApplyCode::Context));}
        assert_eq!(h.records.len(),64);assert!(h.render(701).len()<8192);
        h.dirty=false;h.push(record(100,ApplyCode::Context));assert!(!h.dirty);
        assert_eq!(h.records.back().unwrap().request_id,100);
    }
    #[test] fn unavailable_policy_preserves_readable_roster_but_forbids_changes() {
        // Storage belongs solely to this fixture, not an installed game image.
        let mut game=vec![0u8;alliance::ALLIANCES_OFFSET+12];
        game[alliance::ALLIANCES_OFFSET]=1;
        let mut players=vec![0u8;alliance::PLAYER_SIZE*8];
        players[8]=alliance::HUMAN;
        players[alliance::PLAYER_SIZE..alliance::PLAYER_SIZE+4].copy_from_slice(&1u32.to_le_bytes());
        players[alliance::PLAYER_SIZE+8]=alliance::COMPUTER;
        players[alliance::PLAYER_SIZE+9]=255;
        let config=Config{game:Value::Constant(game.as_ptr() as usize),
            players:Value::Constant(players.as_ptr() as usize),policy:None};
        assert!(config.snapshot(0).unwrap().has_alliance_targets());
        assert!(!config.allowed());
    }
    #[test] fn policy_still_requires_ordinary_game_and_absent_matchmaking() {
        let mut data=vec![0u8;0x80];data[0x7c]=1;
        let string=vec![0u8;40];
        let policy=Policy{game_data:Value::Constant(data.as_ptr() as usize),
            matcher_count:Value::Constant(0),matcher_string:Value::Constant(string.as_ptr() as usize)};
        assert!(policy.allowed());
        data[0x7f]=1;assert!(!policy.allowed());
        data[0x7f]=0;data[0x7c]=0;assert!(!policy.allowed());
        let policy=Policy{matcher_count:Value::Constant(1),..policy};
        data[0x7c]=1;assert!(!policy.allowed());
    }
    #[test] fn scr_visibility_event_abi_matches_pinned_x64_control_event() {
        assert_eq!(std::mem::size_of::<ControlEvent>(),40);
        assert_eq!(std::mem::align_of::<ControlEvent>(),8);
        assert_eq!(std::mem::offset_of!(ControlEvent,ext_type),0);
        assert_eq!(std::mem::offset_of!(ControlEvent,ext_param),8);
        assert_eq!(std::mem::offset_of!(ControlEvent,param),16);
        assert_eq!(std::mem::offset_of!(ControlEvent,ty),24);
        assert_eq!(std::mem::offset_of!(ControlEvent,x),26);
        assert_eq!(std::mem::offset_of!(ControlEvent,y),28);
        assert_eq!(std::mem::offset_of!(ControlEvent,time),32);
    }
    fn owned_snapshot()->alliance::Snapshot {
        let players=std::array::from_fn(|slot|alliance::Player {
            slot:slot as u8,id:slot as u32,storm_id:slot as u32,
            controller:if slot<2{alliance::HUMAN}else{alliance::COMPUTER},
            race:(slot%3) as u8,team:(slot%5) as u8,
            name:[0;alliance::PLAYER_NAME_SIZE],palette_index:None,
        });
        alliance::Snapshot {owner:0,players,alliance_row:[1,2,0,1,2,0,0,0,1,1,1,1]}
    }
    fn owned_edits()->requests::Edits {
        requests::parse(b"SCALLYEDIT1\t1\t3\t4\t50\t8\t1\nE\t2\t0\t1").unwrap()
    }
    fn owned_discovery()->dialog::Discovery {
        let area=dialog::Rect{left:20,top:30,right:279,bottom:389};
        let button=|id,control,top|dialog::Button {
            id,control,area:dialog::Rect{left:10,top,right:100,bottom:top+20},enabled:true,
        };
        dialog::Discovery {
            target:dialog::Target{control:0x10000,slot_address:0x10060,callback:0x20000},
            frame:dialog::Frame {
                area,visible_human_rows:1,confirm:button(-2,0x10100,310),
                cancel:button(-3,0x10200,310),
                allied_victory:dialog::Rect{left:10,top:280,right:200,bottom:300},
                available_rows:Vec::new(),
            },
        }
    }
    fn owned_diplomacy()->Diplomacy {
        Diplomacy {
            session:3,generation:4,frame:50,in_game:true,owner:0,session_boundary:0,identity:Some([0,1,1]),
            target:Some(owned_discovery()),snapshot:Some(owned_snapshot()),
            staged:Some(owned_edits()),last_request:8,ack:0,allowed:true,
            area:[0;4],canvas:[0;2],message:"fixture",pending_apply:None,confirmation:None,
        }
    }
    fn owned_capture()->Capture {
        Capture {
            buffer:BufferSnapshot{bytes:vec![1,2],buffer:0x30000,capacity:512},
            context:(true,50,0),session:[0,1,1],generation:4,
            binding_epoch:6,reentry_epoch:9,edits:owned_edits(),snapshot:owned_snapshot(),
        }
    }
    #[test] fn widescreen_ui_canvas_uses_console_endpoints_not_zoomed_world() {
        let text="ROOT\t0\t0\t1\t1\t0\t315\t137\t479\tMinimap\nROOT\t0\t0\t1\t1\t708\t354\t851\t479\tStatBtn\nROOT\t0\t0\t1\t1\t433\t0\t852\t19\tStatRes\n";
        assert_eq!(canvas_from_metadata(text),Some([853,480]));
        assert_eq!(canvas_from_metadata(&text.replace("StatBtn","Unknown")),None);
        assert_eq!(canvas_from_metadata(&text.replace("479","999")),None);
    }
    #[test] fn capture_checks_previous_frame_owner_and_session_before_refreshing_stage() {
        let d=owned_diplomacy();let found=owned_discovery();
        assert!(staged_context_matches(&d,(true,50,0),[0,1,1],&found));
        assert!(staged_context_matches(&d,(true,51,0),[0,1,1],&found));
        for context in [(false,50,0),(true,49,0),(true,50,1),(true,50,255)] {
            assert!(!staged_context_matches(&d,context,[0,1,1],&found));
        }
        assert!(!staged_context_matches(&d,(true,50,0),[0,1,2],&found));
        let mut d=owned_diplomacy();d.in_game=false;
        assert!(!staged_context_matches(&d,(true,50,0),[0,1,1],&found));
        d=owned_diplomacy();d.allowed=false;
        assert!(!staged_context_matches(&d,(true,50,0),[0,1,1],&found));
    }
    #[test] fn changed_child_layout_or_callback_does_not_reuse_cached_permission() {
        let d=owned_diplomacy();let mut found=owned_discovery();
        found.frame.confirm.enabled=false;
        assert!(!staged_context_matches(&d,(true,50,0),[0,1,1],&found));
        found=owned_discovery();found.frame.visible_human_rows=2;
        assert!(!staged_context_matches(&d,(true,50,0),[0,1,1],&found));
        found=owned_discovery();found.target.callback+=8;
        assert!(!staged_context_matches(&d,(true,50,0),[0,1,1],&found));
    }
    #[test] fn unchanged_computer_mask_is_insufficient_after_target_relation_changes() {
        let before=owned_snapshot();let edits=owned_edits();let mut now=before.clone();
        assert!(snapshot_stable(&before,&now,&edits));
        now.alliance_row[2]=1;
        assert_eq!(now.computer_mask(),before.computer_mask());
        assert!(!snapshot_stable(&before,&now,&edits));
        now=before.clone();now.alliance_row[11]=0;
        assert!(!snapshot_stable(&before,&now,&edits));
        now=before.clone();now.alliance_row[1]=0;
        assert!(!snapshot_stable(&before,&now,&edits));
    }
    #[test] fn same_computer_mask_cannot_hide_changed_roster_or_wrong_owner() {
        let before=owned_snapshot();let edits=owned_edits();
        let mut changed=before.clone();changed.players[2].storm_id+=1;
        assert_eq!(changed.computer_mask(),before.computer_mask());
        assert!(!snapshot_stable(&before,&changed,&edits));
        changed=before.clone();changed.players[2].id=7;
        assert!(!snapshot_stable(&before,&changed,&edits));
        changed=before.clone();changed.players[1].controller=0;
        assert_eq!(changed.computer_mask(),before.computer_mask());
        assert!(!snapshot_stable(&before,&changed,&edits));
        changed=before.clone();changed.owner=1;
        assert!(!snapshot_stable(&before,&changed,&edits));
    }
    #[test] fn registration_changes_and_nested_dispatch_invalidate_outer_capture() {
        let capture=owned_capture();
        assert!(capture_epoch_matches(&capture,6,9,false));
        assert!(!capture_epoch_matches(&capture,7,9,false));
        assert!(!capture_epoch_matches(&capture,6,10,false));
        assert!(!capture_epoch_matches(&capture,6,9,true));
    }
    #[test] fn post_original_request_identity_must_still_match_the_exact_staged_edit() {
        let capture=owned_capture();let mut d=owned_diplomacy();
        assert!(request_still_staged(&d,&capture));
        d.staged=None;assert!(!request_still_staged(&d,&capture));
        d=owned_diplomacy();d.staged.as_mut().unwrap().request_id+=1;
        assert!(!request_still_staged(&d,&capture));
        d=owned_diplomacy();d.generation+=1;
        assert!(!request_still_staged(&d,&capture));
        d=owned_diplomacy();d.session+=1;
        assert!(!request_still_staged(&d,&capture));
        d=owned_diplomacy();d.identity=Some([0,1,2]);
        assert!(!request_still_staged(&d,&capture));
        d=owned_diplomacy();d.allowed=false;
        assert!(!request_still_staged(&d,&capture));
    }
    #[test] fn old_pid_reused_file_and_future_timestamps_cannot_stage_valid_metadata() {
        let epoch=SystemTime::UNIX_EPOCH+Duration::from_secs(100);
        let now=epoch+Duration::from_secs(20);
        // The payload can be syntactically valid and share the process's early
        // session/generation numbers. Its pre-initialization timestamp still
        // prevents use when Windows later reuses that PID.
        assert!(requests::validate(&owned_edits(),&owned_snapshot()));
        assert!(!request_timestamp_valid(epoch-Duration::from_millis(1),epoch,now));
        assert!(request_timestamp_valid(epoch,epoch,now));
        assert!(request_timestamp_valid(now,epoch,now));
        assert!(!request_timestamp_valid(now+Duration::from_millis(1),epoch,now));
        assert!(!request_timestamp_valid(epoch,epoch,epoch-Duration::from_secs(1)));
    }
    #[test] fn unsupported_relationship_rows_are_excluded_from_both_layout_and_output() {
        let mut snapshot=owned_snapshot();
        assert_eq!(editable_computer_count(&snapshot),6);
        snapshot.alliance_row[2]=3;snapshot.alliance_row[7]=3;
        assert_eq!(snapshot.computer_mask().count_ones(),6);
        assert_eq!(editable_computer_count(&snapshot),4);
        let serialized_slots:Vec<_>=snapshot.players.iter()
            .filter(|player|editable_computer(&snapshot,player.slot))
            .map(|player|player.slot).collect();
        assert_eq!(serialized_slots,vec![3,4,5,6]);
        assert_eq!(serialized_slots.len(),editable_computer_count(&snapshot));
        assert_eq!(snapshot.command().unwrap()[0],alliance::ALLIANCE_COMMAND);
    }
    #[test] fn cancel_close_invalidates_staged_request_on_ui_thread_before_worker_poll() {
        let mut d=owned_diplomacy();
        assert!(invalidate_changed_dialog(&mut d,0x10000,None));
        assert_eq!(d.generation,5);assert!(d.staged.is_none());assert!(d.target.is_none());
        assert!(!d.allowed);assert_eq!(d.message,"Alliance dialog closed; staged edits not confirmed");
        assert!(!invalidate_changed_dialog(&mut d,0x10000,None));
        assert_eq!(d.generation,5);
    }
    #[test] fn immediate_root_address_reuse_with_new_callback_or_layout_is_new_generation() {
        let mut d=owned_diplomacy();let mut reused=owned_discovery();
        reused.target.callback+=8;
        assert!(invalidate_changed_dialog(&mut d,0x10000,Some(&reused)));
        assert!(d.staged.is_none());assert_eq!(d.generation,5);
        d=owned_diplomacy();reused=owned_discovery();reused.frame.confirm.control+=8;
        assert!(invalidate_changed_dialog(&mut d,0x10000,Some(&reused)));
        assert_eq!(d.generation,5);
    }
    #[test] fn generic_mouse_callback_with_unchanged_dialog_does_not_discard_stage() {
        let mut d=owned_diplomacy();let same=owned_discovery();
        assert!(!invalidate_changed_dialog(&mut d,0x10000,Some(&same)));
        assert_eq!(d.staged,Some(owned_edits()));assert_eq!(d.generation,4);
        assert!(!invalidate_changed_dialog(&mut d,0x99900,None));
        assert_eq!(d.staged,Some(owned_edits()));
    }
    #[test] fn native_close_preserves_confirm_append_diagnostic_for_worker_to_publish() {
        let mut d=owned_diplomacy();d.staged=None;d.ack=8;
        d.message="Alliance command appended; awaiting actual game state";
        assert!(invalidate_changed_dialog(&mut d,0x10000,None));
        assert_eq!(d.ack,8);
        assert_eq!(d.message,"Alliance command appended; awaiting actual game state");
        assert!(terminal_diagnostic(d.message));
        assert!(!terminal_diagnostic("Alliance dialog ready"));
        assert!(!terminal_diagnostic("Alliance edits staged; use native Confirm"));
    }
    #[test] fn worker_frame_progress_preserves_sample_but_rollback_owner_or_live_changes_do_not() {
        let before=Some((true,50,0));
        assert!(context_continues(before,before));
        assert!(context_continues(before,Some((true,51,0))));
        assert!(context_continues(Some((true,51,0)),Some((true,52,0))));
        let d=owned_diplomacy();let found=owned_discovery();
        assert!(staged_context_matches(&d,(true,52,0),[0,1,1],&found));
        assert_eq!(d.generation,4);assert_eq!(d.staged,Some(owned_edits()));
        for after in [None,Some((false,51,0)),Some((true,49,0)),Some((true,51,1)),Some((true,51,255))] {
            assert!(!context_continues(before,after));
        }
        assert!(!context_continues(Some((false,50,0)),Some((true,51,0))));
        assert!(!context_continues(Some((true,51,0)),Some((true,50,0))));
    }
    #[test] fn stop_discards_staged_edit_without_allowing_the_same_request_to_stage_again() {
        let mut d=owned_diplomacy();let edits=owned_edits();
        assert!(clear_staged_on_stop(&mut d));
        assert!(d.staged.is_none());assert_eq!(d.last_request,edits.request_id);
        assert_eq!(d.generation,5);
        assert!(edits.request_id<=d.last_request);
        assert!(!request_still_staged(&d,&owned_capture()));
        assert!(!clear_staged_on_stop(&mut d));assert_eq!(d.generation,5);
        assert_eq!(d.ack,0);
        assert_eq!(d.message,"Alliance edits cancelled by stop request");
        assert!(terminal_diagnostic(d.message));
        // Even an in-memory draft with an older request watermark keeps its
        // just-discarded request consumed rather than enabling a file replay.
        d=owned_diplomacy();d.last_request=0;
        assert!(clear_staged_on_stop(&mut d));assert_eq!(d.last_request,edits.request_id);
    }
    #[test] fn owned_auto_reset_event_is_resignalled_for_the_existing_pump_after_capture_peek() {
        struct OwnedEvent(*mut c_void);
        impl Drop for OwnedEvent {
            fn drop(&mut self){unsafe{winapi::um::handleapi::CloseHandle(self.0);}}
        }
        // An unnamed event owned by this test only; no game process or existing
        // launcher event is opened, signalled, or consumed.
        let event=OwnedEvent(unsafe{CreateEventW(null_mut(),0,0,null())});
        assert!(!event.0.is_null());
        assert_ne!(unsafe{SetEvent(event.0)},0);
        let mut d=owned_diplomacy();
        let peek=unsafe{WaitForSingleObject(event.0,0)};
        assert_eq!(peek,0);
        assert!(!stop_wait_allows_capture(peek,||{clear_staged_on_stop(&mut d);},
            ||unsafe{SetEvent(event.0)}!=0));
        assert!(d.staged.is_none());
        // This wait represents the unchanged unit pump receiving its stop.
        assert_eq!(unsafe{WaitForSingleObject(event.0,0)},0);
        assert_eq!(unsafe{WaitForSingleObject(event.0,0)},0x102);
        assert!(stop_wait_allows_capture(0x102,||panic!("timeout must not cancel"),
            ||panic!("timeout must not signal")));
        assert!(!stop_wait_allows_capture(u32::MAX,||panic!("invalid wait cannot cancel"),
            ||panic!("invalid wait cannot signal")));
    }
}
