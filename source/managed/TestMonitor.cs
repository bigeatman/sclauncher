using System;
using System.Diagnostics;
using System.IO;
using System.Security.Cryptography;
using System.Text;
using System.Threading.Tasks;

namespace ScMultiTest
{
    internal sealed class TestMonitor
    {
        private const string SupportedHash = "CE3AB05DC9A6AA35418947E5D95E19651AD6FBCD5E2C1856CD729C5B7C31281C";
        internal Snapshot Current { get; private set; }
        internal UnitVisuals CurrentVisuals { get; private set; }
        internal AllianceSnapshot CurrentAlliance { get; private set; }
        internal string AllianceStatus { get; private set; }
        internal ulong AllianceRequestSequence { get; private set; }
        internal IntPtr Window { get; private set; }
        internal string Status { get; private set; }
        internal int Pid { get; private set; }
        internal bool Connecting { get { return attachTask != null && !attachTask.IsCompleted; } }
        internal bool IntegrityPending { get { return Connecting || (verificationTask != null && !verificationTask.IsCompleted); } }
        internal bool IntegrityFailed { get { return verificationTask != null && verificationTask.IsCompleted && verificationTask.Result.Length != 0; } }
        internal string AuditPath { get { return originals == null ? null : originals.LastAuditPath; } }
        internal string IntegrityStatus
        {
            get
            {
                OriginalFiles snapshot = originals;
                if (snapshot == null) return Connecting ? "원본 실행 파일·DLL 보존본 생성 및 해시 확인 중" : "원본 설치 폴더에는 쓰지 않습니다. 연결 전에 외부 보존본을 만듭니다.";
                if (verificationTask == null || !verificationTask.IsCompleted) return "원본 " + snapshot.FileCount + "개 보존본 확인됨 · 스타 종료 후 다시 검사합니다.";
                string[] errors = verificationTask.Result;
                return errors.Length == 0 ? "종료 후 검사 완료 · 원본 실행 파일·DLL 및 보존본 모두 일치" : "원본/보존본 검사 실패: " + errors[0];
            }
        }
        internal static string DataDirectory
        {
            get
            {
                string local = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
                if (String.IsNullOrWhiteSpace(local)) throw new InvalidOperationException("개인 데이터 저장 폴더를 찾을 수 없습니다.");
                return Path.Combine(local, "SCMultiTest");
            }
        }
        private DateTime started;
        private Task<string> attachTask;
        private volatile Task<string[]> verificationTask;
        private volatile OriginalFiles originals;
        private volatile bool suspended;
        private string attachError;
        private Snapshot lastNative;
        private bool requested;
        internal TestMonitor() { Status = "스타 실행 후 '게임 연결'을 눌러 주세요."; }

        internal void Connect()
        {
            if (Connecting) return;
            Process[] games = new Process[0];
            try
            {
                games = Process.GetProcessesByName("StarCraft");
                if (games.Length != 1) throw new InvalidOperationException(games.Length == 0 ? "스타크래프트를 먼저 실행하세요." : "스타크래프트가 여러 개 실행 중입니다. 하나만 남겨 주세요.");
                Process target = games[0];
                DateTime processStart = target.StartTime.ToUniversalTime();
                if (requested && Pid == target.Id && started == processStart)
                {
                    ExistingConnectionAction action = DecideExistingConnection(attachError, lastNative == null ? null : lastNative.State);
                    if (action == ExistingConnectionAction.RestartGame)
                    {
                        Current = lastNative;
                        Status = "모듈 연결에 실패했습니다. 스타를 완전히 종료 후 다시 실행하세요.";
                    }
                    else
                    {
                        suspended = false;
                        Status = action == ExistingConnectionAction.ResumeReady
                            ? "기존 게임 연결을 재개합니다."
                            : "모듈 응답 대기 · 연결 성공은 아직 확인되지 않았습니다.";
                    }
                    return;
                }
                if (IntegrityPending) throw new InvalidOperationException("이전 스타 종료 및 원본 검사가 끝날 때까지 기다려 주세요.");
                Pid = target.Id; started = processStart; requested = true; Current = null; CurrentAlliance = null; AllianceStatus = null; attachError = null; lastNative = null;
                AllianceRequestSequence = 0;
                suspended = false; originals = null; verificationTask = null;
                int targetPid = Pid; DateTime expectedStart = started;
                attachTask = Task.Factory.StartNew(delegate
                {
                    try
                    {
                        using (Process game = Process.GetProcessById(targetPid))
                        {
                            if (game.StartTime.ToUniversalTime() != expectedStart) return "게임 프로세스가 변경되었습니다.";
                            using (var sha = SHA256.Create())
                            using (var stream = File.OpenRead(game.MainModule.FileName))
                                if (BitConverter.ToString(sha.ComputeHash(stream)).Replace("-", "") != SupportedHash)
                                    return "대상 게임 빌드가 아닙니다. 기준은 1.23.10.13515 x64입니다.";
                            Native.CheckExistingModule(game, Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "sc_multi_test.dll"));
                            OriginalFiles saved = OriginalFiles.Capture(game.MainModule.FileName, AppDomain.CurrentDomain.BaseDirectory, DataDirectory);
                            originals = saved;
                            if (suspended || game.HasExited || game.StartTime.ToUniversalTime() != expectedStart)
                            {
                                verificationTask = Task.Factory.StartNew(delegate { return SafeVerify(saved); });
                                return "연결 요청이 취소되었거나 게임이 종료되었습니다.";
                            }
                            try { Native.Attach(game, Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "sc_multi_test.dll")); }
                            finally
                            {
                                // An attempted load can time out after the module has started.
                                // Keep the exact process under observation until it exits.
                                verificationTask = Task.Factory.StartNew(delegate { return VerifyAfterGame(saved, targetPid, expectedStart); });
                            }
                            return null;
                        }
                    }
                    catch (Exception ex)
                    {
                        OriginalFiles saved = originals;
                        if (saved != null && verificationTask == null)
                            verificationTask = Task.Factory.StartNew(delegate { return SafeVerify(saved); });
                        return ex.Message;
                    }
                });
                Status = "원본 보존본 및 게임 버전 확인 후 모듈 연결 중";
            }
            catch (Exception ex) { Status = "연결 실패: " + ex.Message; }
            finally { foreach (Process process in games) process.Dispose(); }
        }
        internal enum ExistingConnectionAction { RestartGame, ResumeReady, WaitForModule }
        internal static ExistingConnectionAction DecideExistingConnection(string attachFailure, string nativeState)
        {
            if (attachFailure != null || nativeState == "FAILED") return ExistingConnectionAction.RestartGame;
            return nativeState == "READY" ? ExistingConnectionAction.ResumeReady : ExistingConnectionAction.WaitForModule;
        }
        private string NativeFailureStatus()
        {
            return "모듈 연결에 실패했습니다. 스타를 완전히 종료 후 다시 실행하세요. 원인: " + StatusText.Explain(lastNative.Message);
        }
        private static string[] VerifyAfterGame(OriginalFiles saved, int pid, DateTime expectedStart)
        {
            try
            {
                using (Process game = Process.GetProcessById(pid))
                    if (!game.HasExited && game.StartTime.ToUniversalTime() == expectedStart) game.WaitForExit();
            }
            catch (ArgumentException) { }
            catch (InvalidOperationException) { }
            catch (Exception ex) { return new[] { "게임 종료 확인 실패: " + ex.Message }; }
            return SafeVerify(saved);
        }
        private static string[] SafeVerify(OriginalFiles saved)
        {
            try { return saved.Verify(); }
            catch (Exception ex) { return new[] { "원본 검사 실패: " + ex.Message }; }
        }
        internal void Tick()
        {
            Current = null; CurrentVisuals = null; CurrentAlliance = null; AllianceStatus = null; Window = IntPtr.Zero;
            if (!requested) return;
            try
            {
                using (Process game = Process.GetProcessById(Pid))
                {
                    if (game.HasExited || game.StartTime.ToUniversalTime() != started) { Reset(); return; }
                    if (!suspended) Native.Heartbeat(Pid);
                    Window = game.MainWindowHandle;
                    if (Connecting) { Status = "원본 보존본 및 게임 버전 확인 후 모듈 연결 중"; return; }
                    if (attachTask != null) attachError = attachTask.Result;
                    if (attachError != null) { Status = "연결 실패: " + attachError; return; }
                    // A failed native initialization cannot be retried in this process.
                    // Keep the failure even after a stopped session clears Current.
                    if (lastNative != null && lastNative.State == "FAILED")
                    {
                        Current = lastNative;
                        Status = NativeFailureStatus();
                        return;
                    }
                    if (suspended) { Status = "전체 제어 중지 · 스타 종료 후 원본 검사 대기"; return; }
                    string file = Path.Combine(DataDirectory, "mc-" + Pid + ".tsv");
                    if (!File.Exists(file)) { Status = "모듈 응답 대기 · 연결 성공은 아직 확인되지 않았습니다."; return; }
                    DateTime modified = File.GetLastWriteTimeUtc(file);
                    if (!Snapshot.IsFresh(modified, started, DateTime.UtcNow))
                    { Status = "상태 갱신이 끊겼습니다. 게임을 재시작한 뒤 다시 연결하세요."; return; }
                    Current = Snapshot.Parse(SnapshotFile.Read(file), Pid);
                    lastNative = Current;
                    if (Current.State == "READY" && Current.InGame)
                    {
                        DateTime nowUtc = DateTime.UtcNow;
                        CurrentAlliance = AllianceEditsFile.ReadCurrent(Path.Combine(DataDirectory, "mc-" + Pid + "-alliance.tsv"), Pid, started, nowUtc);
                        if (CurrentAlliance != null)
                        {
                            AllianceStatus = AllianceEditsFile.ReadStatus(Path.Combine(DataDirectory, "mc-" + Pid + "-alliance-status.txt"), started, nowUtc);
                            AllianceRequestSequence = Math.Max(AllianceRequestSequence, Math.Max((ulong)nowUtc.Ticks,
                                Math.Max(CurrentAlliance.AckRequestId, AllianceEditsFile.ReadApplySequenceFloor(
                                    Path.Combine(DataDirectory, "mc-" + Pid + "-alliance-apply.tsv"), CurrentAlliance, started, nowUtc))));
                        }
                    }
                    if (Current.State == "READY" && Current.Active)
                        CurrentVisuals = UnitVisuals.Read(Path.Combine(DataDirectory, "mc-" + Pid + "-visuals.tsv"), Pid, started, DateTime.UtcNow);
                    if (Current.State == "FAILED") Status = NativeFailureStatus();
                    else if (Current.State == "WAITING") Status = "모듈 준비 중: " + StatusText.Explain(Current.Message);
                    else if (Current.Active) Status = String.Format("선택된 유닛: 총 {0}개 · 버퍼에 추가된 묶음 {1:N0}개", Current.Count, Current.AppendedBatches);
                    else Status = "연결됨 · 게임에서 유닛 선택 후 백틱(`)을 누르세요.";
                }
            }
            catch (ArgumentException) { Reset(); }
            catch (Exception ex) { Current = null; CurrentAlliance = null; AllianceStatus = null; Status = "상태 확인 실패: " + ex.Message; }
        }
        private void Reset()
        {
            Current = null; CurrentVisuals = null; CurrentAlliance = null; AllianceStatus = null; Pid = 0; requested = false; attachError = null; lastNative = null; Window = IntPtr.Zero;
            AllianceRequestSequence = 0;
            // Capture may still be copying after the game exits. Retain the task so
            // the form cannot exit or start another session before its final audit.
            Status = "게임이 종료되었습니다. 원본 검사 결과를 확인하세요.";
        }
        internal void SuspendControl()
        {
            suspended = true;
            CurrentAlliance = null; AllianceStatus = null;
            try { if (requested && Pid > 0) Stop(); } catch { }
        }
        internal void PublishAllianceEdits(AllianceEditsEventArgs edits)
        {
            if (!requested || suspended || Connecting || Pid <= 0 || Current == null || Current.State != "READY" || !Current.InGame)
                throw new InvalidOperationException("연결된 게임에서 동맹창을 열어 주세요.");
            AllianceSnapshot latest = AllianceEditsFile.ReadCurrent(Path.Combine(DataDirectory, "mc-" + Pid + "-alliance.tsv"), Pid, started, DateTime.UtcNow);
            if (latest == null) throw new InvalidOperationException("컴퓨터 동맹 상태 갱신이 끊겼습니다. 동맹창을 다시 열어 주세요.");
            AllianceEditsFile.Publish(DataDirectory, edits, latest);
            CurrentAlliance = latest;
        }
        internal void PublishAllianceApply(AllianceApplyEventArgs request)
        {
            if (!requested || suspended || Connecting || Pid <= 0 || request == null || request.Pid != Pid ||
                Current == null || Current.State != "READY" || !Current.InGame)
                throw new InvalidOperationException("연결된 게임에서 동맹창을 열어 주세요.");
            DateTime nowUtc = DateTime.UtcNow;
            string currentPath = Path.Combine(DataDirectory, "mc-" + Pid + ".tsv");
            if (!File.Exists(currentPath) || !Snapshot.IsFresh(File.GetLastWriteTimeUtc(currentPath), started, nowUtc))
                throw new InvalidOperationException("게임 상태 갱신이 끊겨 컴퓨터 동맹 변경을 보내지 않았습니다.");
            Snapshot live = Snapshot.Parse(SnapshotFile.Read(currentPath), Pid);
            if (live.State != "READY" || !live.InGame ||
                !Snapshot.IsFresh(File.GetLastWriteTimeUtc(currentPath), started, DateTime.UtcNow))
                throw new InvalidOperationException("현재 게임 상태에서는 컴퓨터 동맹 변경을 보낼 수 없습니다.");
            AllianceSnapshot latest = AllianceEditsFile.ReadCurrent(Path.Combine(DataDirectory, "mc-" + Pid + "-alliance.tsv"), Pid, started, DateTime.UtcNow);
            if (latest == null || !latest.Open)
                throw new InvalidOperationException("컴퓨터 동맹 상태 갱신이 끊겼습니다. 동맹창을 다시 열어 주세요.");
            if (!latest.Allowed)
                throw new InvalidOperationException("동맹 변경 허용 상태를 확인하지 못했거나 현재 게임에서 변경이 금지되어 요청을 보내지 않았습니다.");
            AllianceEditsFile.PublishApply(DataDirectory, request, latest);
            CurrentAlliance = latest;
            AllianceRequestSequence = Math.Max(AllianceRequestSequence, request.RequestId);
        }
        internal void Stop()
        {
            if (!requested) throw new InvalidOperationException("연결된 게임이 없습니다.");
            using (Process game = Process.GetProcessById(Pid))
            {
                if (game.HasExited || game.StartTime.ToUniversalTime() != started) throw new InvalidOperationException("게임이 종료되거나 변경되었습니다.");
                Native.StopSelection(Pid);
            }
        }
    }
    internal static class SnapshotFile
    {
        private const int MaximumBytes = 4096;
        internal static FileStream Open(string path)
        {
            // The native writer publishes by replacing the old file atomically.
            return new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
        }
        internal static string Read(string path)
        {
            using (FileStream input = Open(path)) return ReadText(input);
        }
        internal static string ReadText(Stream input)
        {
            // Limit bytes actually read, even if a shared file grows after it is opened.
            byte[] bytes = new byte[MaximumBytes + 1];
            int length = 0;
            while (length < bytes.Length)
            {
                int read = input.Read(bytes, length, bytes.Length - length);
                if (read == 0) break;
                length += read;
            }
            if (length > MaximumBytes) throw new FormatException("상태 파일 크기 초과");
            using (var buffered = new MemoryStream(bytes, 0, length, false))
            using (var reader = new StreamReader(buffered, new UTF8Encoding(false, true))) return reader.ReadToEnd();
        }
    }
}
