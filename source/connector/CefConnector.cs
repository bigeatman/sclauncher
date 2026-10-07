using System;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Management;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Security.Principal;
using System.Text.RegularExpressions;
using System.Threading.Tasks;

internal static class CefConnector {
 const string HelperHash="828f893f316699fa4be22a20542dd1258c53d0bf71a8379ab19b8ba9cf3d5c57";
 const string CefHash="f780bf312a2a265edefb48e13dce358245724d665db18a27af4ffbb94404498d";
 [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint access,bool inherit,int id);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool IsWow64Process(IntPtr process,out bool wow64);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetProcessTimes(IntPtr process,out long created,out long exited,out long kernel,out long user);
 [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr VirtualAllocEx(IntPtr process,IntPtr address,UIntPtr bytes,uint allocation,uint protection);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool VirtualFreeEx(IntPtr process,IntPtr address,UIntPtr bytes,uint operation);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool WriteProcessMemory(IntPtr process,IntPtr address,byte[] buffer,UIntPtr size,out UIntPtr written);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode,ExactSpelling=true)] static extern IntPtr GetModuleHandleW(string name);
 [DllImport("kernel32.dll",CharSet=CharSet.Ansi,ExactSpelling=true)] static extern IntPtr GetProcAddress(IntPtr module,string name);
 [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr CreateRemoteThread(IntPtr process,IntPtr attr,UIntPtr stack,IntPtr start,IntPtr parameter,uint flags,IntPtr tid);
 [DllImport("kernel32.dll",SetLastError=true)] static extern uint WaitForSingleObject(IntPtr handle,uint timeout);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetExitCodeThread(IntPtr thread,out uint code);
 [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetExitCodeProcess(IntPtr process,out uint code);
 [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
 static string Hash(string path) { using(var file=File.OpenRead(path)) using(var sha=SHA256.Create()) return BitConverter.ToString(sha.ComputeHash(file)).Replace("-", "").ToLowerInvariant(); }
 static bool Same(string a,string b) { return string.Equals(Path.GetFullPath(a),Path.GetFullPath(b),StringComparison.OrdinalIgnoreCase); }
 static void Assert(bool test,string msg) { if(!test) throw new InvalidOperationException(msg); }
 static void Write(string msg) { Console.WriteLine(DateTime.Now.ToString("HH:mm:ss")+"  "+msg); }
 static int Main(string[] args) {
  Console.OutputEncoding=new UTF8Encoding(false);
  try {
   Assert(Environment.Is64BitProcess,"Connector must run x64.");
   if(args.Contains("--status-selftest"))return StatusSelfTest();
   bool waiting=args.Contains("--wait");
   StartupIdentity startup=waiting?WaitForStartup(args):null;
   bool connect=args.Contains("--connect") || waiting;
   Assert(connect || args.Contains("--probe"),"Use --probe, --connect --dll <external DLL>, or --wait --dll <external DLL> before launching the game.");
   var games=Process.GetProcessesByName("StarCraft"); Assert(games.Length==1,"Exactly one running StarCraft is required.");
   using(var game=games[0]) {
    string gamePath=game.MainModule.FileName;
    Assert(game.MainModule.FileVersionInfo.FileVersion.StartsWith("1.23.10.13515",StringComparison.Ordinal),"Unsupported game build.");
    Assert(string.Equals(Path.GetFileName(Path.GetDirectoryName(gamePath)),"x86_64",StringComparison.OrdinalIgnoreCase),"x64 game path required.");
    string install=Path.GetDirectoryName(Path.GetDirectoryName(gamePath));
    string helperPath=Path.Combine(Path.GetDirectoryName(gamePath),"cef","SceneCefBrowser.exe");
    string cefPath=Path.Combine(Path.GetDirectoryName(helperPath),"libcef.dll");
    Assert(waiting || (Hash(helperPath)==HelperHash && Hash(cefPath)==CefHash),"Unsupported official CEF files; originals not changed by connector.");
    if(waiting)Assert(game.Id==startup.GameId && Same(gamePath,startup.GamePath),"Startup game identity changed.");
    int helperId=waiting?startup.HelperId:0; int count=waiting?1:0;
    if(!waiting)
    using(var search=new ManagementObjectSearcher("SELECT ProcessId,ParentProcessId,CommandLine FROM Win32_Process WHERE Name='SceneCefBrowser.exe'"))
    using(var results=search.Get()) foreach(ManagementObject row in results) {
     using(row) {
      string command=(string)row["CommandLine"] ?? "";
      if(Convert.ToInt32(row["ParentProcessId"])!=game.Id) continue;
      string token="SyncMemName=SceneBrowserSyncMem."+game.Id;
      if(!command.Split(new[]{' ', '\t'},StringSplitOptions.RemoveEmptyEntries).Contains(token)) continue;
      helperId=Convert.ToInt32(row["ProcessId"]); count++;
     }
    }
    Assert(count==1,"Exactly one official CEF browser child is required.");
    using(var helper=Process.GetProcessById(helperId)) {
     Assert(Same(helper.MainModule.FileName,helperPath),"Unexpected helper path.");
     Write("Verified game PID="+game.Id+", CEF browser PID="+helper.Id+", build=1.23.10.13515 x64.");
     if(!connect) { Write("Probe complete: process metadata and file hashes only; no module attached."); return 0; }
     int arg=Array.IndexOf(args,"--dll"); Assert(arg>=0 && arg+1<args.Length,"DLL path is required.");
     string dll=Path.GetFullPath(args[arg+1]);
     Assert(string.Equals(Path.GetFileName(dll),"sc_lobby_ui.dll",StringComparison.OrdinalIgnoreCase),"Expected sc_lobby_ui.dll.");
     Assert(!dll.StartsWith(install+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase),"DLL must remain outside the installation.");
     Assert(File.Exists(dll) && File.Exists(dll+".sha256"),"DLL and SHA-256 manifest are required.");
     Assert(Hash(dll)==File.ReadAllText(dll+".sha256").Trim().ToLowerInvariant(),"External module hash mismatch.");
     string script=Path.Combine(Path.GetDirectoryName(dll),"bootstrap.js");
     Assert(File.Exists(script) && File.Exists(script+".sha256"),"Bootstrap and SHA-256 manifest are required.");
     Assert(Hash(script)==File.ReadAllText(script+".sha256").Trim().ToLowerInvariant(),"Bootstrap hash mismatch.");
     foreach(ProcessModule module in helper.Modules) if(string.Equals(module.ModuleName,"sc_lobby_ui.dll",StringComparison.OrdinalIgnoreCase)) {
      Assert(Hash(module.FileName)==Hash(dll),"Another lobby module is already loaded; restart game before switching builds.");
      Write("This module is already loaded; restarting the local status monitor."); using(var existingMonitor=new StatusMonitor(helper.Id)){existingMonitor.Prepare();existingMonitor.Run();} return 0;
     }
     string deployment=Deploy(dll,script,helper.Id);
     using(var monitor=new StatusMonitor(helper.Id)) {
      monitor.Prepare();
      Task monitorTask=Task.Run(()=>monitor.Run());
      try {
       Attach(helper,deployment);
       Assert(Hash(helperPath)==HelperHash && Hash(cefPath)==CefHash,"Official file integrity check failed after attachment.");
       Write("External module load completed. Waiting for lobby acknowledgement; this connector stays open until game exit.");
       monitorTask.GetAwaiter().GetResult();
      } catch { monitor.Stop();monitorTask.GetAwaiter().GetResult();throw; }
     }
    }
   }
   return 0;
  } catch(Exception ex) { Console.Error.WriteLine("FAILED: "+(args.Contains("--status-selftest")?ex.ToString():ex.Message)); return 1; }
 }
 sealed class StartupIdentity{public int GameId,HelperId;public string GamePath;}
 static StartupIdentity WaitForStartup(string[] args) {
  Assert(Process.GetProcessesByName("StarCraft").Length==0,"Startup capture requires closing StarCraft first. Game originals are never overwritten.");
  int arg=Array.IndexOf(args,"--game");
  string path=arg>=0 && arg+1<args.Length?Path.GetFullPath(args[arg+1]):Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFilesX86),"StarCraft","x86_64","StarCraft.exe");
  Assert(File.Exists(path),"Official StarCraft path not found. Use --game <official x64 StarCraft.exe path>.");
  Assert(FileVersionInfo.GetVersionInfo(path).FileVersion.StartsWith("1.23.10.13515",StringComparison.Ordinal),"Unsupported game build.");
  string cefFolder=Path.Combine(Path.GetDirectoryName(path),"cef");
  Assert(Hash(Path.Combine(cefFolder,"SceneCefBrowser.exe"))==HelperHash && Hash(Path.Combine(cefFolder,"libcef.dll"))==CefHash,"Unsupported official CEF files.");
  Write("STARTUP_WAIT: official files verified. Launch StarCraft normally from Battle.net; waiting up to 120 seconds.");
  var timeout=Stopwatch.StartNew();
  while(timeout.ElapsedMilliseconds<120000) {
   var games=Process.GetProcessesByName("StarCraft");
   try {
    Assert(games.Length<=1,"Multiple StarCraft processes appeared; startup capture stopped.");
    if(games.Length==1 && Same(games[0].MainModule.FileName,path)) {
     var children=Process.GetProcessesByName("SceneCefBrowser");
     try {
      if(children.Length>0) {
       using(var search=new ManagementObjectSearcher("SELECT ProcessId,ParentProcessId,CommandLine FROM Win32_Process WHERE Name='SceneCefBrowser.exe'"))
       using(var results=search.Get()) foreach(ManagementObject row in results) using(row) {
        if(Convert.ToInt32(row["ParentProcessId"])!=games[0].Id)continue;
        string command=(string)row["CommandLine"]??"";
        if(!command.Split(new[]{' ','\t'},StringSplitOptions.RemoveEmptyEntries).Contains("SyncMemName=SceneBrowserSyncMem."+games[0].Id))continue;
        return new StartupIdentity{GameId=games[0].Id,HelperId=Convert.ToInt32(row["ProcessId"]),GamePath=path};
       }
      }
     }finally{foreach(var child in children)child.Dispose();}
    }
   }catch(Win32Exception){/* Module metadata may not be published on the first startup tick. */}
   finally{foreach(var game in games)game.Dispose();}
   System.Threading.Thread.Sleep(20);
  }
  throw new InvalidOperationException("Startup wait timed out. No game files were changed.");
 }
 static void Attach(Process target,string dll) {
  long expected=target.StartTime.ToUniversalTime().ToFileTimeUtc();
  IntPtr process=OpenProcess(0x0002|0x0400|0x0008|0x0010|0x0020,false,target.Id);
  if(process==IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
  IntPtr remote=IntPtr.Zero,thread=IntPtr.Zero; bool finished=false;
  try {
   long created,exited,kernel,user; bool wow64;
   if(!GetProcessTimes(process,out created,out exited,out kernel,out user)) throw new Win32Exception(Marshal.GetLastWin32Error());
   Assert(created==expected,"Helper process identity changed.");
   Assert(IsWow64Process(process,out wow64) && !wow64,"Expected x64 helper.");
   IntPtr local=GetProcAddress(GetModuleHandleW("kernel32.dll"),"LoadLibraryW"); Assert(local!=IntPtr.Zero,"Windows loader unavailable.");
   string owner=null; long rva=0;
   using(var self=Process.GetCurrentProcess()) foreach(ProcessModule module in self.Modules) {
    long baseAddress=module.BaseAddress.ToInt64();
    if(local.ToInt64()>=baseAddress && local.ToInt64()<baseAddress+module.ModuleMemorySize) { owner=module.ModuleName; rva=local.ToInt64()-baseAddress; break; }
   }
   IntPtr loader=IntPtr.Zero;
   foreach(ProcessModule module in target.Modules) if(string.Equals(module.ModuleName,owner,StringComparison.OrdinalIgnoreCase)) { loader=new IntPtr(module.BaseAddress.ToInt64()+rva); break; }
   Assert(loader!=IntPtr.Zero,"Target Windows loader unavailable.");
   byte[] bytes=Encoding.Unicode.GetBytes(dll+"\0");
   remote=VirtualAllocEx(process,IntPtr.Zero,new UIntPtr((uint)bytes.Length),0x1000|0x2000,0x04);
   if(remote==IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
   UIntPtr written;
   if(!WriteProcessMemory(process,remote,bytes,new UIntPtr((uint)bytes.Length),out written) || written.ToUInt64()!=(ulong)bytes.Length) throw new Win32Exception(Marshal.GetLastWin32Error());
   thread=CreateRemoteThread(process,IntPtr.Zero,UIntPtr.Zero,loader,remote,0,IntPtr.Zero);
   if(thread==IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
   finished=WaitForSingleObject(thread,8000)==0;
   Assert(finished,"Module load timed out; restart game before retrying.");
   uint code; if(!GetExitCodeThread(thread,out code))throw new Win32Exception(Marshal.GetLastWin32Error());if(code==0)throw new InvalidOperationException("External module was not loaded from the separate deployment folder.");
  } finally {
   if(thread!=IntPtr.Zero) CloseHandle(thread);
   if(remote!=IntPtr.Zero && (thread==IntPtr.Zero || finished)) VirtualFreeEx(process,remote,UIntPtr.Zero,0x8000);
   CloseHandle(process);
  }
 }
 static string Deploy(string dll,string script,int pid) {
  string folder=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData),"SCMultiTestLobby","runtime",pid+"-"+Hash(dll).Substring(0,16));
  Directory.CreateDirectory(folder);
  foreach(string source in new[]{dll,script,dll+".sha256",script+".sha256"}) {
   string destination=Path.Combine(folder,Path.GetFileName(source));
   if(File.Exists(destination))Assert(Hash(source)==Hash(destination),"Deployment file differs; close game before changing module.");
   else File.Copy(source,destination,false);
  }
  Write("Module staged outside installation: "+folder);
  return Path.Combine(folder,Path.GetFileName(dll));
 }
 static string ReadSharedLog(string path){using(var file=new FileStream(path,FileMode.Open,FileAccess.Read,FileShare.ReadWrite))using(var reader=new StreamReader(file))return reader.ReadToEnd();}
 static int StatusSelfTest() {
  int id=Process.GetCurrentProcess().Id;
  string name="SCMultiTestLobby.Status."+id;
  using(var monitor=new StatusMonitor(id)) {
   monitor.Prepare();Task task=Task.Run(()=>monitor.Run());
   try {
    foreach(string text in new[]{"SELFTEST_A","invalid","SELFTEST_B","SELFTEST_C"}) {
     using(var client=new System.IO.Pipes.NamedPipeClientStream(".",name,System.IO.Pipes.PipeDirection.Out)) {
      client.Connect(2000);byte[] bytes=Encoding.ASCII.GetBytes(text=="invalid"?"not a valid status\n":"12:34:56 pid="+id+" "+text+" value=1\r\n");client.Write(bytes,0,bytes.Length);client.Flush();
     }
    }
    string expected="SELFTEST_C";var deadline=Stopwatch.StartNew();
    while(deadline.ElapsedMilliseconds<2000 && (!File.Exists(monitor.Logfile) || !ReadSharedLog(monitor.Logfile).Contains(expected)))System.Threading.Thread.Sleep(20);
    string result=ReadSharedLog(monitor.Logfile);
    Assert(result.Contains("SELFTEST_A") && result.Contains("SELFTEST_B") && result.Contains("SELFTEST_C") && !result.Contains("not a valid status"),"IPC self-test failed: messages were dropped or malformed text was accepted.");
    monitor.Stop();Assert(task.Wait(2000),"IPC monitor did not stop.");
    Write("Status self-test passed: queued connections, three messages, rejected malformed input, cancellation.");return 0;
   } finally {monitor.Stop();task.Wait(2000);}
  }
 }
 sealed class StatusMonitor:IDisposable {
  [StructLayout(LayoutKind.Sequential)] struct SecurityAttributes { public int Length;public IntPtr Descriptor;public int Inherit; }
  [StructLayout(LayoutKind.Sequential)] struct Overlapped {public IntPtr Internal,InternalHigh;public uint Offset,OffsetHigh;public IntPtr Event;}
  [DllImport("advapi32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool ConvertStringSecurityDescriptorToSecurityDescriptor(string text,uint revision,out IntPtr descriptor,IntPtr size);
  [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);
  [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr CreateNamedPipe(string name,uint access,uint mode,uint instances,uint output,uint input,uint timeout,ref SecurityAttributes security);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool ConnectNamedPipe(IntPtr pipe,IntPtr overlapped);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool DisconnectNamedPipe(IntPtr pipe);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool ReadFile(IntPtr file,IntPtr buffer,uint count,out uint bytes,IntPtr overlapped);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetOverlappedResult(IntPtr file,IntPtr overlapped,out uint bytes,bool wait);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool GetNamedPipeClientProcessId(IntPtr pipe,out uint pid);
  [DllImport("kernel32.dll",SetLastError=true)] static extern bool CancelIoEx(IntPtr file,IntPtr overlapped);
  [DllImport("kernel32.dll",SetLastError=true)] static extern IntPtr CreateEvent(IntPtr attributes,bool manualReset,bool initial,string name);
  [DllImport("kernel32.dll")] static extern bool SetEvent(IntPtr handle);
  [DllImport("kernel32.dll")] static extern bool ResetEvent(IntPtr handle);
  [DllImport("kernel32.dll")] static extern uint WaitForMultipleObjects(uint count,IntPtr[] handles,bool all,uint timeout);
  readonly int pid;readonly string logfile;readonly string pipeName;
  IntPtr descriptor=IntPtr.Zero,pipe=IntPtr.Zero,doneEvent=IntPtr.Zero,stopEvent=IntPtr.Zero,process=IntPtr.Zero,operation=IntPtr.Zero,buffer=IntPtr.Zero; bool pending;
  public string Logfile{get{return logfile;}}
  public StatusMonitor(int processId){pid=processId;string folder=Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"SCMultiTestLobby");Directory.CreateDirectory(folder);logfile=Path.Combine(folder,"loader-status-"+pid+".txt");pipeName=@"\\.\pipe\SCMultiTestLobby.Status."+pid;}
  public void Prepare(){
   string sid=WindowsIdentity.GetCurrent().User.Value;
   // New inbound-only status IPC with a low integrity label; no existing ACL or system policy is changed.
   string sddl="D:(A;;GA;;;"+sid+")(A;;GRGW;;;BU)S:(ML;;NW;;;LW)";
   if(!ConvertStringSecurityDescriptorToSecurityDescriptor(sddl,1,out descriptor,IntPtr.Zero))throw new Win32Exception(Marshal.GetLastWin32Error(),"Status descriptor creation failed.");
   process=OpenProcess(0x00100000|0x1000,false,pid);if(process==IntPtr.Zero)throw new Win32Exception(Marshal.GetLastWin32Error());
   doneEvent=CreateEvent(IntPtr.Zero,true,false,null);stopEvent=CreateEvent(IntPtr.Zero,true,false,null);
   if(doneEvent==IntPtr.Zero || stopEvent==IntPtr.Zero)throw new Win32Exception(Marshal.GetLastWin32Error());
   // These buffers must remain at fixed addresses for the entire pending I/O operation.
   operation=Marshal.AllocHGlobal(Marshal.SizeOf(typeof(Overlapped)));buffer=Marshal.AllocHGlobal(256);
   File.WriteAllText(logfile,"");CreatePipe();
  }
  void ResetOperation(){ResetEvent(doneEvent);Marshal.StructureToPtr(new Overlapped{Event=doneEvent},operation,false);}
  void CreatePipe(){var security=new SecurityAttributes{Length=Marshal.SizeOf(typeof(SecurityAttributes)),Descriptor=descriptor,Inherit=0};
   pipe=CreateNamedPipe(pipeName,1|0x00080000|0x40000000,4|2|8,1,256,256,0,ref security);
   if(pipe==new IntPtr(-1)){pipe=IntPtr.Zero;throw new Win32Exception(Marshal.GetLastWin32Error(),"Local status endpoint unavailable.");}
   BeginConnect();
  }
  void BeginConnect(){
   ResetOperation();bool connected=ConnectNamedPipe(pipe,operation);int error=connected?0:Marshal.GetLastWin32Error();pending=!connected && error==997;
   if(!connected && error!=997 && error!=535)throw new InvalidOperationException("Status connection setup failed: win32="+error);
  }
  bool Finish(out uint bytes){
   bytes=0;uint result=WaitForMultipleObjects(3,new[]{doneEvent,process,stopEvent},false,0xffffffff);
   if(result!=0){CancelIoEx(pipe,operation);GetOverlappedResult(pipe,operation,out bytes,true);return false;}
   return GetOverlappedResult(pipe,operation,out bytes,false);
  }
  public void Run(){
   while(WaitForSingleObject(process,0)!=0 && WaitForSingleObject(stopEvent,0)!=0){
    uint transferred=0;
    if(pending && !Finish(out transferred)) {
     if(WaitForSingleObject(process,0)==0 || WaitForSingleObject(stopEvent,0)==0){ReportExit();return;}
    }else {
     uint client;bool verified=GetNamedPipeClientProcessId(pipe,out client) && client==(uint)pid;
     if(verified){
      ResetOperation();bool read=ReadFile(pipe,buffer,256,out transferred,operation);int error=read?0:Marshal.GetLastWin32Error();
      if(!read && error==997)read=Finish(out transferred);
      if(read && transferred>0 && transferred<=256){
       byte[] data=new byte[transferred];Marshal.Copy(buffer,data,0,(int)transferred);string line=Encoding.ASCII.GetString(data);
       if(Regex.IsMatch(line,@"^\d{2}:\d{2}:\d{2} pid="+pid+@" [A-Z_]+ value=-?\d+\r?\n$")){using(var log=new FileStream(logfile,FileMode.Append,FileAccess.Write,FileShare.ReadWrite))using(var writer=new StreamWriter(log,Encoding.ASCII)){writer.Write(line);}Console.Write(line);}
      }
     }
    }
    DisconnectNamedPipe(pipe);
    if(WaitForSingleObject(process,0)==0 || WaitForSingleObject(stopEvent,0)==0){ReportExit();return;}
    BeginConnect();
   }
  }
  void ReportExit(){uint code;if(WaitForSingleObject(process,0)==0 && GetExitCodeProcess(process,out code))Write("CEF_HELPER_EXIT PID="+pid+" value="+code+" hex=0x"+code.ToString("x8"));}
  public void Stop(){if(stopEvent!=IntPtr.Zero)SetEvent(stopEvent);}
  public void Dispose(){
   Stop();if(pipe!=IntPtr.Zero){CancelIoEx(pipe,IntPtr.Zero);CloseHandle(pipe);}
   if(operation!=IntPtr.Zero)Marshal.FreeHGlobal(operation);if(buffer!=IntPtr.Zero)Marshal.FreeHGlobal(buffer);
   if(doneEvent!=IntPtr.Zero)CloseHandle(doneEvent);if(stopEvent!=IntPtr.Zero)CloseHandle(stopEvent);
   if(process!=IntPtr.Zero)CloseHandle(process);if(descriptor!=IntPtr.Zero)LocalFree(descriptor);
  }
 }
}