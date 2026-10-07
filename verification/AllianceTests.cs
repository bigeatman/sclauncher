using System;
using System.Drawing;
using ScMultiTest;
internal static class Native { }
namespace ScMultiTest { internal static class Native { internal static bool SetWindowPos(IntPtr a, IntPtr b, int x, int y, int w, int h, uint flags) { return true; } } }
internal static class AllianceTests {
 private static int count;
 private const string H = "SCALLY1\t701\t10\t20\t100\t1\t1\t200\t200\t500\t320\t640\t480\t0";
 private static readonly DateTime Now = new DateTime(2026,10,6,0,0,0,DateTimeKind.Utc);
 private static AllianceSnapshot P(string value) { return AllianceSnapshot.Parse(value,701); }
 private static AllianceSnapshot S(int relation, ulong ack, uint frame) { string[] h=H.Split('\t'); h[4]=frame.ToString(); h[13]=ack.ToString(); return P(String.Join("\t",h)+"\nC\t6\t8\t"+relation); }
 private static AllianceSnapshot NativeRows(int visibleHumans, int areaRows, int computers) {
  // Target-build fixture: root (276,21), first label (11,46), pitch 26;
  // runtime row area adds -2 above and +6 below its label endpoints.
  string header="SCALLY1\t701\t10\t20\t100\t1\t1\t287\t"+(65+26*visibleHumans)+"\t524\t"+(66+26*(visibleHumans+areaRows))+"\t853\t480\t0";
  for(int i=0;i<computers;i++)header+="\nC\t"+i+"\t16\t0";
  return P(header);
 }
 private static void Check(bool value,string name) { if(!value) throw new Exception(name); count++; }
 private static void Reject(string value,string name) { try { P(value); } catch(FormatException) { count++; return; } throw new Exception("invalid accepted: "+name); }
 private static void Main() { try { Run(); Console.WriteLine("Alliance managed checks passed: "+count); } catch(Exception e) { Console.Error.WriteLine(e); Environment.ExitCode=1; } }
 private static void Run() {
  AllianceSnapshot x=P(H+"\nC\t6\t8\t0\nC\t7\t15\t2");
  Check(x.Pid==701 && x.Session==10 && x.Generation==20 && x.Frame==100 && x.Computers.Count==2,"header and rows");
  Check(x.RowArea==new Rectangle(200,200,300,120) && x.CanvasWidth==640 && x.CanvasHeight==480,"explicit row area and canvas");
  Check(!x.Computers[0].Allied && x.Computers[1].Allied,"relation 0/2");
  Check(P(H+"\r\nC\t6\t8\t0\r\n").Computers.Count==1,"CRLF wire");
  Check(P(H+"\n").Computers.Count==0,"empty roster");
  Reject(null,"null"); Reject("", "empty"); Reject(new string('x',2049),"oversize");
  Reject(H.Replace("701","702"),"wrong PID"); Reject(H.Replace("SCALLY1","SCALLY2"),"version");
  Reject(H.Replace("\t701\t","\t+701\t"),"signed number"); Reject(H+"\textra","extra header field");
  Reject(H+"\r","bare CR"); Reject(H+"\n\n","extra line"); Reject(H+"\0","nul");
  Reject(H+"\nC\t8\t0\t0","neutral slot"); Reject(H+"\nC\t0\t17\t0","color range");
  Check(P(H+"\nC\t0\t16\t0").Computers[0].ColorIndex==16,"unknown color retained as gray");
  Reject(H+"\nC\t0\t0\t3","relation range"); Reject(H+"\nH\t0\t0\t0","human row injection");
  Reject(H+"\nC\t6\t8\t0\nC\t6\t8\t0","duplicate slot"); Reject(H+"\nC\t7\t8\t0\nC\t6\t8\t0","unsorted slots");
  string eight=H; for(int i=0;i<8;i++)eight+="\nC\t"+i+"\t0\t0"; Reject(eight,"eighth computer");
  Reject(H.Replace("\t500\t","\t641\t"),"out of canvas"); Reject(H.Replace("\t640\t","\t0\t"),"one zero canvas dimension");
  Reject(H.Replace("\t10\t20\t","\t0\t20\t"),"open zero session");
  Check(AllianceSnapshot.IsFresh(Now,Now.AddSeconds(-1),Now.AddSeconds(2)),"fresh");
  Check(!AllianceSnapshot.IsFresh(Now,Now.AddSeconds(1),Now.AddSeconds(2)),"predates process");
  Check(!AllianceSnapshot.IsFresh(Now,Now.AddSeconds(-1),Now.AddSeconds(3)),"stale");
  Check(!AllianceSnapshot.IsFresh(Now.AddSeconds(1),Now.AddSeconds(-1),Now),"future timestamp");
  AllianceOverlayLayout l;
  Check(AllianceOverlayLayout.TryCreate(x,new Rectangle(10,20,1920,1200),true,out l) && l.Bounds==new Rectangle(610,520,900,300),"canvas coordinate transform");
  Check(l.Rows.Bottom<=l.Status.Top && l.Status.Bottom<l.Apply.Top && l.Apply.Bottom<=l.Bounds.Height,"non-overlapping rows and footer");
  Check(l.Apply.Right<l.Cancel.Left && l.Cancel.Right<l.Bounds.Width,"separated action buttons");
  Check(!AllianceOverlayLayout.TryCreate(x,new Rectangle(0,0,200,100),out l),"small client hidden");
  Check(!AllianceOverlayLayout.TryCreate(P(H.Replace("\t1\t1\t200","\t0\t1\t200")),new Rectangle(0,0,1920,1200),out l),"closed native dialog hidden");
  Check(!AllianceOverlayLayout.TryCreate(P(H),new Rectangle(0,0,1920,1200),out l),"no computers hidden");
  foreach(int height in new int[]{768,840}) {
   int humans=height==768?1:2;
   Check(AllianceOverlayLayout.TryCreate(NativeRows(humans,5,5),new Rectangle(0,0,1366,height),out l),"fractional native layout created "+height);
   Check(l.VisibleRows==5 && l.Apply.IsEmpty && l.Cancel.IsEmpty,"fractional scale keeps all five native rows "+height);
   for(int i=0;i<5;i++) {
    Rectangle row=l.Row(i);
    Check(row.Top==(int)Math.Round(i*26*height/480.0) && row.Bottom==(int)Math.Round((i+1)*26*height/480.0),"independently rounded native row boundaries "+height+" "+i);
    Check(l.RowIndexAt(new Point(row.Left,row.Top))==i && l.RowIndexAt(new Point(row.Right-1,row.Bottom-1))==i,"fractional paint and hit test agree "+height+" "+i);
   }
   Check(l.Row(4).Bottom==l.Rows.Bottom && l.RowIndexAt(new Point(0,l.Rows.Bottom))==-1,"native final row and outside hit test "+height);
   Check(l.Status.IsEmpty,"unreadable fractional status remainder omitted "+height);
  }
  Check(AllianceOverlayLayout.TryCreate(NativeRows(3,4,4),new Rectangle(0,0,1920,1200),out l) && l.VisibleRows==4 && l.RowHeight==65 && l.Rows.Height==260,"1200px native alignment and pitch preserved");
  Check(l.Bounds.Bottom<(int)Math.Round(281*2.5) && l.Status.IsEmpty,"native panel remains above allied victory and suppresses tiny status");
  Check(AllianceOverlayLayout.TryCreate(NativeRows(0,2,5),new Rectangle(0,0,1920,1200),out l) && l.VisibleRows==1 && l.Status.Height>=12,"scrolling page reserves readable clickable arrows");
  Check(!AllianceOverlayLayout.TryCreate(NativeRows(0,1,5),new Rectangle(0,0,1920,1200),out l),"one-row scrolling panel without arrow space hidden");
  var e=new AllianceEditor(); e.Update(x,Now); Check(e.Toggle(6) && e.Checked(6) && e.Dirty,"pending computer toggle");
  Check(!e.Toggle(0),"human slot not editable");
  AllianceApplyEventArgs r=e.PrepareRequest(); Check(r.Pid==701 && r.Session==10 && r.Generation==20 && r.RequestId==1 && r.Changes.Count==1 && r.Changes[0].Slot==6 && r.Changes[0].ExpectedRelation==0 && r.Changes[0].DesiredRelation==1,"request preserves context and only changes computer");
  r.Error="fixture I/O failure"; e.FinishDispatch(r,Now); Check(!e.Busy && e.Dirty && e.Notice==r.Error,"failed dispatch preserves pending edits");
  r=e.PrepareRequest(); r.AcceptedForDispatch=true; e.FinishDispatch(r,Now); Check(e.Busy && !e.Toggle(6) && !e.CancelPending(),"dispatched edits lock until verified");
  e.Update(P(H.Replace("\t0", "\t0")+"\nC\t6\t8\t0\nC\t7\t15\t2"),Now.AddSeconds(1)); Check(e.Busy,"no ACK does not confirm");
  string ackHeader=H.Substring(0,H.LastIndexOf('\t'))+"\t"+r.RequestId;
  e.Update(P(ackHeader+"\nC\t6\t8\t0\nC\t7\t15\t2"),Now.AddSeconds(2)); Check(e.Busy,"ACK alone does not confirm relation");
  e.Update(P(ackHeader+"\nC\t6\t8\t1\nC\t7\t15\t2"),Now.AddSeconds(3)); Check(!e.Busy && !e.Dirty && e.Checked(6),"ACK and actual relation confirm");
  Check(e.Toggle(7) && !e.Checked(7) && e.Toggle(7) && e.Checked(7) && !e.Dirty,"allied-victory relation untouched after toggle back");
  e.Toggle(6); e.Update(P(ackHeader.Replace("\t20\t", "\t21\t")+"\nC\t6\t8\t1\nC\t7\t15\t2"),Now.AddSeconds(4)); Check(!e.Dirty,"generation change clears pending edit");
  e.Toggle(6); e.Update(P(ackHeader.Replace("\t20\t", "\t21\t")+"\nC\t6\t8\t0\nC\t7\t15\t2"),Now.AddSeconds(5)); Check(!e.Dirty && !e.Checked(6),"external relationship change clears pending edit");
  e.Toggle(6); e.Update(P(ackHeader.Replace("\t20\t", "\t21\t")+"\nC\t6\t9\t0\nC\t7\t15\t2"),Now.AddSeconds(6)); Check(!e.Dirty,"roster color change clears pending edit");
  var timeout=new AllianceEditor(); timeout.Update(S(0,0,100),Now); timeout.Toggle(6); r=timeout.PrepareRequest(); r.AcceptedForDispatch=true; timeout.FinishDispatch(r,Now);
  timeout.Update(S(0,r.RequestId,101),Now.AddSeconds(5)); Check(!timeout.Busy && !timeout.Checked(6) && timeout.Notice!=null,"unmatched acknowledged relation fails instead of showing success");
  var stale=new AllianceEditor(); stale.Update(S(0,10,100),Now); stale.Toggle(6); stale.Update(S(1,10,99),Now.AddSeconds(1)); Check(stale.Dirty && stale.Current.Frame==100 && stale.Current.Computers[0].Relation==0,"older frame rejected");
  stale.Update(S(1,9,101),Now.AddSeconds(1)); Check(stale.Dirty && stale.Current.Frame==100,"older ACK rejected");
  r=stale.PrepareRequest(); Check(r.RequestId==11,"request IDs advance past native ACK");
  var locked=new AllianceEditor(); locked.Update(P(H.Replace("\t1\t1\t200", "\t1\t0\t200")+"\nC\t6\t8\t0"),Now); Check(!locked.Toggle(6) && locked.PrepareRequest()==null,"locked mode cannot dispatch");
  var rollback=new AllianceEditor(); rollback.Update(x,Now); rollback.Toggle(6);
  AllianceEditsEventArgs successful=rollback.PrepareEdits(); successful.Published=true; rollback.FinishPublish(successful);
  rollback.Toggle(6); rollback.Toggle(7);
  AllianceEditsEventArgs failedNewer=rollback.PrepareEdits(); failedNewer.Error="fixture replacement failure"; rollback.FinishPublish(failedNewer);
  Check(rollback.Checked(6) && rollback.Checked(7) && rollback.PendingChanges.Count==1 && rollback.PendingChanges[0].Slot==6 && rollback.PendingChanges[0].DesiredRelation==1,"failed newer publication restores full last successfully staged diff");
  Check(!rollback.Busy && rollback.Current.FindComputer(6).Relation==0 && rollback.Notice.Contains("실패") && rollback.Notice.Contains("되돌"),"rollback is pending UI state with explicit failure notice");
  rollback.Toggle(7); AllianceEditsEventArgs retried=rollback.PrepareEdits(); retried.Published=true; rollback.FinishPublish(retried);
  string rollbackHeader=H.Substring(0,H.LastIndexOf('\t'))+"\t"+retried.RequestId;
  rollback.Update(P(rollbackHeader+"\nC\t6\t8\t1\nC\t7\t15\t0"),Now.AddSeconds(1));
  Check(!rollback.Dirty && rollback.Checked(6) && !rollback.Checked(7),"successful retry after rollback confirms only fresh matching actual relations");
  var noPublished=new AllianceEditor(); noPublished.Update(S(0,0,100),Now); noPublished.Toggle(6);
  AllianceEditsEventArgs firstFailed=noPublished.PrepareEdits(); firstFailed.Error="fixture first publication failure"; noPublished.FinishPublish(firstFailed);
  Check(!noPublished.Checked(6) && !noPublished.Dirty && noPublished.PendingChanges.Count==0 && noPublished.Notice.Contains("되돌"),"failed first publication resets to actual game relation");
  noPublished.Toggle(6); AllianceEditsEventArgs firstRetry=noPublished.PrepareEdits(); firstRetry.Published=true; noPublished.FinishPublish(firstRetry);
  noPublished.Update(S(1,firstRetry.RequestId,101),Now.AddSeconds(1));
  Check(!noPublished.Dirty && noPublished.Checked(6) && noPublished.Notice==null,"successful retry after first-write failure confirms normally");
  var restarted=new AllianceEditor(); restarted.Update(S(0,10,100),Now); restarted.AdvanceRequestSequence(72); restarted.Toggle(6);
  AllianceEditsEventArgs afterRestart=restarted.PrepareEdits();
  Check(afterRestart.RequestId==73 && afterRestart.RequestId>72 && afterRestart.RequestId>restarted.Current.AckRequestId,"launcher restart seeds above persisted consumed native ID and last committed ACK");
  restarted.AdvanceRequestSequence(12); Check(restarted.PrepareEdits().RequestId==74,"lower subsequent sequence floor cannot move request IDs backwards");
  restarted.AdvanceRequestSequence((ulong)Now.Ticks); Check(restarted.PrepareEdits().RequestId==(ulong)Now.Ticks+1,"host clock sequence seed does not alter pure default constructor IDs");
  var exhausted=new AllianceEditor(); exhausted.Update(S(0,0,100),Now); exhausted.AdvanceRequestSequence(UInt64.MaxValue);
  Check(!exhausted.Toggle(6) && exhausted.PrepareEdits()==null && exhausted.PrepareRequest()==null && exhausted.Notice!=null,"maximum persisted request sequence fails closed without overflow");
  e.Update(null,Now); Check(e.Current==null && !e.Dirty && !e.Busy,"missing snapshot clears UI state");
  var immediate=new AllianceEditor(); immediate.Update(S(0,0,100),Now);
  AllianceApplyEventArgs sent=null; int dispatches=0;
  Action<AllianceApplyEventArgs> accepted=delegate(AllianceApplyEventArgs request){sent=request; dispatches++; request.AcceptedForDispatch=true;};
  Check(AllianceOverlayForm.DispatchImmediateToggle(immediate,6,accepted,Now),"checkbox dispatches without an Apply button or native Confirm");
  Check(dispatches==1 && sent.Changes.Count==1 && sent.Changes[0].Slot==6 && sent.Changes[0].ExpectedRelation==0 && sent.Changes[0].DesiredRelation==1,"immediate checkbox submits only clicked computer");
  Check(immediate.Busy && immediate.Checked(6) && immediate.Current.FindComputer(6).Relation==0,"accepted transport retains pending UI and observed state separately");
  Check(!AllianceOverlayForm.DispatchImmediateToggle(immediate,6,accepted,Now.AddSeconds(1)) && dispatches==1,"busy checkbox cannot duplicate in-flight command");
  Check(!immediate.CancelPending() && immediate.Busy,"Cancel cannot undo an already dispatched immediate command");
  immediate.Update(S(1,0,101),Now.AddSeconds(1)); Check(immediate.Busy,"actual immediate relation alone is not dispatch acknowledgement");
  immediate.Update(S(0,sent.RequestId,102),Now.AddSeconds(2)); Check(immediate.Busy,"immediate ACK alone is not actual application");
  immediate.Update(S(1,sent.RequestId,103),Now.AddSeconds(3)); Check(!immediate.Busy && !immediate.Dirty && immediate.Checked(6),"immediate checkbox completes only on ACK and matching actual relation");
  Check(AllianceOverlayForm.DispatchImmediateToggle(immediate,6,accepted,Now.AddSeconds(4)) && sent.Changes[0].ExpectedRelation==1 && sent.Changes[0].DesiredRelation==0,"next immediate click can revoke the committed own-player relation");
  var immediateFailure=new AllianceEditor(); immediateFailure.Update(S(0,0,100),Now);
  Check(AllianceOverlayForm.DispatchImmediateToggle(immediateFailure,6,delegate(AllianceApplyEventArgs request){request.Error="fixture apply delivery denied";},Now),"delivery denial handled after checkbox click");
  Check(!immediateFailure.Busy && !immediateFailure.Dirty && !immediateFailure.Checked(6) && immediateFailure.Notice=="fixture apply delivery denied","failed immediate delivery restores actual unchecked relation and reports failure");
  Check(AllianceOverlayForm.DispatchImmediateToggle(immediateFailure,6,delegate(AllianceApplyEventArgs request){throw new InvalidOperationException("fixture apply exception");},Now),"throwing immediate transport handled without a window");
  Check(!immediateFailure.Busy && !immediateFailure.Checked(6) && immediateFailure.Notice=="fixture apply exception","throwing immediate transport cannot leave a false checked state");
  Check(AllianceOverlayForm.DispatchImmediateToggle(immediateFailure,6,null,Now) && !immediateFailure.Checked(6) && !immediateFailure.Dirty && !immediateFailure.Busy,"missing immediate handler is a failed delivery not a staged edit");
  var immediatePolicy=new AllianceEditor(); immediatePolicy.Update(P(H.Replace("\t1\t1\t200", "\t1\t0\t200")+"\nC\t6\t8\t0"),Now);
  Check(!AllianceOverlayForm.DispatchImmediateToggle(immediatePolicy,6,accepted,Now) && dispatches==2,"forbidden policy cannot emit immediate command");

  // A match ending is not a process restart: retain request sequencing while
  // dropping every pending choice and completion belonging to the old game.
  var consecutiveGames=new AllianceEditor(); consecutiveGames.Update(S(0,10,400),Now);
  Check(consecutiveGames.Toggle(6),"same-PID first game checkbox is editable");
  AllianceApplyEventArgs firstGameRequest=consecutiveGames.PrepareRequest();
  firstGameRequest.AcceptedForDispatch=true; consecutiveGames.FinishDispatch(firstGameRequest,Now);
  Check(consecutiveGames.Busy && consecutiveGames.Checked(6),"same-PID first game has an in-flight checkbox request");
  consecutiveGames.Update(null,Now.AddSeconds(1));
  Check(consecutiveGames.Current==null && !consecutiveGames.Busy && !consecutiveGames.Dirty && consecutiveGames.Notice==null,"menu clears first-game pending checkbox and busy state");
  string[] secondGameHeader=H.Split('\t'); secondGameHeader[2]="11"; secondGameHeader[3]="21"; secondGameHeader[4]="1"; secondGameHeader[13]="0";
  consecutiveGames.Update(P(String.Join("\t",secondGameHeader)+"\nC\t6\t8\t0"),Now.AddSeconds(2));
  Check(consecutiveGames.Current.Pid==firstGameRequest.Pid && consecutiveGames.Current.Session!=firstGameRequest.Session && consecutiveGames.Current.Frame<firstGameRequest.Frame && consecutiveGames.Current.AckRequestId==0,"same-PID second game accepts a new session with lower frame and reset ACK");
  Check(!consecutiveGames.Busy && !consecutiveGames.Dirty && !consecutiveGames.Checked(6) && consecutiveGames.Toggle(6),"second-game actual relation is editable without first-game busy state");
  AllianceApplyEventArgs secondGameRequest=consecutiveGames.PrepareRequest();
  Check(secondGameRequest!=null && secondGameRequest.RequestId>firstGameRequest.RequestId && secondGameRequest.Session==consecutiveGames.Current.Session && secondGameRequest.Generation==consecutiveGames.Current.Generation,"request IDs stay monotonic across games while authorization follows the new session");
  secondGameRequest.AcceptedForDispatch=true; consecutiveGames.FinishDispatch(secondGameRequest,Now.AddSeconds(2));
  string secondGameNotice=consecutiveGames.Notice;
  consecutiveGames.CompleteRequest(firstGameRequest.Session,firstGameRequest.Generation,firstGameRequest.RequestId,false,"stale first-game completion");
  Check(consecutiveGames.Busy && consecutiveGames.Dirty && consecutiveGames.Checked(6) && consecutiveGames.Notice==secondGameNotice,"old completion cannot cancel or replace the second-game in-flight request");
  secondGameHeader[4]="2"; secondGameHeader[13]=secondGameRequest.RequestId.ToString();
  consecutiveGames.Update(P(String.Join("\t",secondGameHeader)+"\nC\t6\t8\t1"),Now.AddSeconds(3));
  Check(!consecutiveGames.Busy && !consecutiveGames.Dirty && consecutiveGames.Checked(6) && consecutiveGames.Notice==null,"second-game request completes on its own ACK and actual relation");
 }
}
