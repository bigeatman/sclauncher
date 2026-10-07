using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;
using ScMultiTest;

internal static class AllianceEditsFileTests
{
    private static int passed;
    private const string Ready = "SCALLY1\t701\t10\t20\t100\t1\t1\t10\t124\t296\t260\t640\t480\t0\nC\t6\t16\t0\nC\t7\t16\t2\n";
    private static void Check(bool value, string name)
    { if (!value) throw new Exception("FAILED: alliance staged file: " + name); passed++; }
    private static AllianceEditsEventArgs Request(AllianceSnapshot current, params AllianceChange[] changes)
    { return new AllianceEditsEventArgs(current, 1, new List<AllianceChange>(changes)); }
    private static AllianceApplyEventArgs ApplyRequest(AllianceSnapshot current, params AllianceChange[] changes)
    { return new AllianceApplyEventArgs(current, 73, new List<AllianceChange>(changes)); }
    private static void RejectApply(AllianceApplyEventArgs request, AllianceSnapshot current, string name)
    {
        try { AllianceEditsFile.SerializeApply(request, current); }
        catch (InvalidOperationException) { passed++; return; }
        throw new Exception("FAILED: invalid immediate apply accepted: " + name);
    }
    private static void Reject(AllianceEditsEventArgs edits, AllianceSnapshot current, string name)
    {
        try { AllianceEditsFile.Serialize(edits, current); }
        catch (InvalidOperationException) { passed++; return; }
        throw new Exception("FAILED: invalid staged edit accepted: " + name);
    }
    private static void Main()
    {
        try { Run(); Console.WriteLine("PASS: " + passed + " computer alliance staged-file / freshness / atomic publication checks"); }
        catch (Exception ex) { Console.Error.WriteLine(ex); Environment.ExitCode = 1; }
    }
    private static void Run()
    {
        AllianceSnapshot current = AllianceSnapshot.Parse(Ready, 701);
        AllianceEditsEventArgs edits = Request(current, new AllianceChange(6, 0, 1), new AllianceChange(7, 2, 0));
        string serialized = "SCALLYEDIT1\t701\t10\t20\t100\t1\t2\nE\t6\t0\t1\nE\t7\t2\t0\n";
        Check(AllianceEditsFile.Serialize(edits, current) == serialized, "exact native protocol and sorted computer-only changes");
        Check(!edits.Published && current.FindComputer(6).Relation == 0, "serialization does not dispatch or mutate state");
        Check(AllianceEditsFile.Serialize(Request(current), current) == "SCALLYEDIT1\t701\t10\t20\t100\t1\t0\n", "empty diff explicitly clears staged changes");
        CultureInfo originalCulture = CultureInfo.CurrentCulture;
        try
        {
            System.Threading.Thread.CurrentThread.CurrentCulture = CultureInfo.GetCultureInfo("ar-SA");
            Check(AllianceEditsFile.Serialize(edits, current) == serialized, "wire numeric fields are culture-independent");
        }
        finally { System.Threading.Thread.CurrentThread.CurrentCulture = originalCulture; }
        Reject(null, current, "missing edits"); Reject(edits, null, "missing current state");
        Reject(edits, AllianceSnapshot.Parse(Ready.Replace("\t701\t", "\t702\t"), 702), "wrong PID");
        Reject(edits, AllianceSnapshot.Parse(Ready.Replace("\t10\t20\t", "\t11\t20\t"), 701), "wrong session");
        Reject(edits, AllianceSnapshot.Parse(Ready.Replace("\t10\t20\t", "\t10\t21\t"), 701), "wrong generation");
        Reject(edits, AllianceSnapshot.Parse(Ready.Replace("\t1\t1\t10\t", "\t0\t1\t10\t"), 701), "native dialog closed");
        Reject(edits, AllianceSnapshot.Parse(Ready.Replace("\t1\t1\t10\t", "\t1\t0\t10\t"), 701), "mode forbids changes");
        Reject(edits, AllianceSnapshot.Parse(Ready.Replace("\t100\t", "\t99\t"), 701), "future request frame");
        Reject(new AllianceEditsEventArgs(current, 0, new List<AllianceChange>()), current, "zero request ID");
        Reject(Request(current, new AllianceChange(8, 0, 1)), current, "neutral slot");
        Reject(Request(current, new AllianceChange(0, 0, 1)), current, "human slot not in computer roster");
        Reject(Request(current, new AllianceChange(6, 1, 0)), current, "expected relation changed");
        Reject(Request(current, new AllianceChange(6, 0, 2)), current, "allied victory cannot be enabled by checkbox");
        Reject(Request(current, new AllianceChange(7, 2, 1)), current, "allied victory cannot be normalized by checkbox");
        Reject(Request(current, new AllianceChange(6, 0, 0)), current, "no-op row");
        Reject(Request(current, new AllianceChange(6, 0, 1), new AllianceChange(6, 0, 1)), current, "duplicate slot");
        Reject(Request(current, new AllianceChange(7, 2, 0), new AllianceChange(6, 0, 1)), current, "unsorted slot");
        AllianceSnapshot ordinaryAlly = AllianceSnapshot.Parse(Ready.Replace("C\t6\t16\t0", "C\t6\t16\t1"), 701);
        Check(AllianceEditsFile.Serialize(Request(ordinaryAlly, new AllianceChange(6, 1, 0)), ordinaryAlly).Contains("E\t6\t1\t0"), "ordinary allied toggle off accepted");
        var many = new List<AllianceChange>(); for (int i = 0; i < 8; i++) many.Add(new AllianceChange(i, 0, 1));
        Reject(new AllianceEditsEventArgs(current, 1, many), current, "too many rows");
        AllianceApplyEventArgs applyRequest = ApplyRequest(current, new AllianceChange(6, 0, 1), new AllianceChange(7, 2, 0));
        string applySerialized = "SCALLYAPPLY1\t701\t10\t20\t100\t73\t2\nE\t6\t0\t1\nE\t7\t2\t0\n";
        Check(AllianceEditsFile.SerializeApply(applyRequest, current) == applySerialized, "immediate command uses exact distinct native protocol");
        Check(!applyRequest.AcceptedForDispatch && current.FindComputer(6).Relation == 0, "apply serialization is not acceptance or actual state");
        RejectApply(null, current, "missing immediate request");
        RejectApply(ApplyRequest(current), current, "empty immediate command cannot clear a staged draft");
        RejectApply(ApplyRequest(current, new AllianceChange[] { null }), current, "missing immediate row rejected explicitly");
        RejectApply(applyRequest, AllianceSnapshot.Parse(Ready.Replace("\t10\t20\t", "\t11\t20\t"), 701), "immediate session changed");
        RejectApply(applyRequest, AllianceSnapshot.Parse(Ready.Replace("\t1\t1\t10\t", "\t1\t0\t10\t"), 701), "immediate policy forbidden");
        RejectApply(ApplyRequest(current, new AllianceChange(0, 0, 1)), current, "immediate human target rejected");
        RejectApply(ApplyRequest(current, new AllianceChange(7, 2, 1)), current, "immediate allied victory normalization rejected");
        RejectApply(ApplyRequest(current, new AllianceChange(7, 2, 0), new AllianceChange(6, 0, 1)), current, "immediate rows require strict sort");

        string directory = Path.Combine(Path.GetTempPath(), "sc-alliance-edits-tests-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        string target = Path.Combine(directory, "mc-701-alliance-edit.tsv");
        string applyTarget = Path.Combine(directory, "mc-701-alliance-apply.tsv");
        string snapshotPath = Path.Combine(directory, "mc-701-alliance.tsv");
        string statusPath = Path.Combine(directory, "mc-701-alliance-status.txt");
        try
        {
            AllianceEditsFile.Publish(directory, edits, current);
            Check(File.ReadAllText(target) == serialized, "new target receives complete staged file");
            byte[] bytes = File.ReadAllBytes(target);
            Check(bytes.Length > 3 && !(bytes[0] == 0xef && bytes[1] == 0xbb && bytes[2] == 0xbf), "request has no UTF8 BOM");
            Check(Directory.GetFiles(directory, "*.tmp").Length == 0, "temporary file removed after new publication");
            bool replacedWhileHeld = false;
            using (var oldReader = new FileStream(target, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))
            {
                try { AllianceEditsFile.Publish(directory, Request(current), current); replacedWhileHeld = true; }
                catch (IOException)
                {
                    // Some Windows filesystem configurations still deny replacing
                    // an open destination despite delete sharing. Preserve the
                    // complete prior file rather than use an in-place fallback.
                    Check(File.ReadAllText(target) == serialized, "delete-shared replacement denial preserves complete previous file");
                }
                using (var reader = new StreamReader(oldReader, Encoding.UTF8, true, 1024, true))
                    Check(reader.ReadToEnd() == serialized, "delete-shared reader retains prior complete file during replacement");
                if (replacedWhileHeld)
                    Check(File.ReadAllText(target).EndsWith("\t0\n", StringComparison.Ordinal), "replacement publishes complete clear request while held");
            }
            if (!replacedWhileHeld) AllianceEditsFile.Publish(directory, Request(current), current);
            Check(File.ReadAllText(target).EndsWith("\t0\n", StringComparison.Ordinal), "atomic replacement succeeds after held reader closes");
            string completeBeforeFailure = File.ReadAllText(target);
            bool failed = false;
            using (var locked = new FileStream(target, FileMode.Open, FileAccess.Read, FileShare.Read))
            {
                try { AllianceEditsFile.Publish(directory, edits, current); }
                catch (IOException) { failed = true; }
                Check(failed, "replacement failure reported when destination denies deletion");
                Check(File.ReadAllText(target) == completeBeforeFailure, "failed publication preserves prior complete file");
            }
            Check(Directory.GetFiles(directory, "*.tmp").Length == 0, "failed publication cleans its temporary file");
            Check(!edits.Published, "writer does not mark unacknowledged edit applied");

            DateTime now = DateTime.UtcNow, started = now.AddMinutes(-1);
            string stagedBeforeApply = File.ReadAllText(target);
            AllianceEditsFile.PublishApply(directory, applyRequest, current);
            Check(File.ReadAllText(applyTarget) == applySerialized, "click publication writes distinct complete apply file");
            Check(File.ReadAllText(target) == stagedBeforeApply, "immediate apply never overwrites legacy staged file");
            Check(!applyRequest.AcceptedForDispatch, "apply writer does not acknowledge its own command");
            File.SetLastWriteTimeUtc(applyTarget, now.AddSeconds(-20));
            Check(AllianceEditsFile.ReadApplySequenceFloor(applyTarget, current, started, now) == 73, "apply restart floor persists throughout game lifetime");
            Check(AllianceEditsFile.ReadSequenceFloor(applyTarget, current, started, now) == 0, "legacy staged reader rejects immediate marker");
            File.WriteAllText(applyTarget, serialized, new UTF8Encoding(false)); File.SetLastWriteTimeUtc(applyTarget, now);
            Check(AllianceEditsFile.ReadApplySequenceFloor(applyTarget, current, started, now) == 0, "old staged payload cannot seed apply sequence even at apply path");
            File.WriteAllText(applyTarget, applySerialized.Replace("\t100\t73\t2", "\t101\t73\t2"), new UTF8Encoding(false)); File.SetLastWriteTimeUtc(applyTarget, now);
            Check(AllianceEditsFile.ReadApplySequenceFloor(applyTarget, current, started, now) == 0, "future apply frame cannot seed sequence");
            File.WriteAllText(applyTarget, "SCALLYAPPLY1\t701\t10\t20\t100\t73\t0\n", new UTF8Encoding(false)); File.SetLastWriteTimeUtc(applyTarget, now);
            Check(AllianceEditsFile.ReadApplySequenceFloor(applyTarget, current, started, now) == 0, "empty immediate payload rejected when reading sequence");
            File.WriteAllText(applyTarget, applySerialized, new UTF8Encoding(false));
            bool applyFailed = false;
            using (var lockedApply = new FileStream(applyTarget, FileMode.Open, FileAccess.Read, FileShare.Read))
            {
                try { AllianceEditsFile.PublishApply(directory, ApplyRequest(current, new AllianceChange(6, 0, 1)), current); }
                catch (IOException) { applyFailed = true; }
                Check(applyFailed && File.ReadAllText(applyTarget) == applySerialized, "apply delivery failure preserves prior complete request");
            }
            Check(Directory.GetFiles(directory, "*.tmp").Length == 0, "failed immediate delivery leaves no temporary files");
            AllianceEditsFile.PublishApply(directory, ApplyRequest(current, new AllianceChange(6, 0, 1)), current);
            Check(File.ReadAllText(applyTarget).EndsWith("\t1\nE\t6\t0\t1\n", StringComparison.Ordinal), "retry publishes complete single-checkbox apply");
            File.WriteAllText(target, serialized.Replace("\t100\t1\t2", "\t100\t72\t2"), new UTF8Encoding(false));
            File.SetLastWriteTimeUtc(target, now.AddSeconds(-20));
            Check(AllianceEditsFile.ReadSequenceFloor(target, current, started, now) == 72, "restart sequence floor persists beyond display freshness window");
            Check(AllianceEditsFile.ReadSequenceFloor(target, AllianceSnapshot.Parse(Ready.Replace("\t10\t20\t", "\t11\t20\t"), 701), started, now) == 0, "persisted floor cannot cross session identity");
            Check(AllianceEditsFile.ReadSequenceFloor(target, AllianceSnapshot.Parse(Ready.Replace("\t10\t20\t", "\t10\t21\t"), 701), started, now) == 0, "persisted floor cannot cross dialog generation");
            Check(AllianceEditsFile.ReadSequenceFloor(target, AllianceSnapshot.Parse(Ready.Replace("\t701\t", "\t702\t"), 702), started, now) == 0, "persisted floor cannot cross PID identity");
            File.SetLastWriteTimeUtc(target, started.AddSeconds(-1));
            Check(AllianceEditsFile.ReadSequenceFloor(target, current, started, now) == 0, "old process request cannot seed current lifetime");
            File.SetLastWriteTimeUtc(target, now.AddSeconds(1));
            Check(AllianceEditsFile.ReadSequenceFloor(target, current, started, now) == 0, "future request timestamp cannot seed sequence");
            File.WriteAllText(target, serialized.Replace("\t100\t1\t2", "\t100\t18446744073709551615\t2")); File.SetLastWriteTimeUtc(target, now);
            Check(AllianceEditsFile.ReadSequenceFloor(target, current, started, now) == UInt64.MaxValue, "maximum sequence retained for fail-closed overflow handling");
            File.WriteAllText(target, serialized.Replace("E\t7\t2\t0", "E\t7\t2\t1")); File.SetLastWriteTimeUtc(target, now);
            Check(AllianceEditsFile.ReadSequenceFloor(target, current, started, now) == 0, "invalid toggle cannot seed sequence");
            File.WriteAllText(target, serialized.Replace("\t1\t2\n", "\t+1\t2\n")); File.SetLastWriteTimeUtc(target, now);
            Check(AllianceEditsFile.ReadSequenceFloor(target, current, started, now) == 0, "signed request ID cannot seed sequence");
            File.WriteAllText(target, serialized.Replace("\t1\t2\n", "\t1\t1\n")); File.SetLastWriteTimeUtc(target, now);
            Check(AllianceEditsFile.ReadSequenceFloor(target, current, started, now) == 0, "row count mismatch cannot seed sequence");
            File.WriteAllText(target, serialized, Encoding.UTF8); File.SetLastWriteTimeUtc(target, now);
            Check(AllianceEditsFile.ReadSequenceFloor(target, current, started, now) == 0, "UTF8 BOM cannot seed ASCII native request sequence");
            File.WriteAllText(target, new string('x',1025)); File.SetLastWriteTimeUtc(target, now);
            Check(AllianceEditsFile.ReadSequenceFloor(target, current, started, now) == 0, "request read bounded at native byte limit");
            File.WriteAllText(snapshotPath, Ready, new UTF8Encoding(false)); File.SetLastWriteTimeUtc(snapshotPath, now);
            Check(AllianceEditsFile.ReadCurrent(snapshotPath, 701, started, now).Computers.Count == 2, "fresh snapshot read");
            Check(AllianceEditsFile.ReadCurrent(snapshotPath, 702, started, now) == null, "wrong PID snapshot hidden");
            File.SetLastWriteTimeUtc(snapshotPath, now.AddSeconds(-3));
            Check(AllianceEditsFile.ReadCurrent(snapshotPath, 701, started, now) == null, "stale snapshot hidden at boundary");
            File.SetLastWriteTimeUtc(snapshotPath, started.AddSeconds(-1));
            Check(AllianceEditsFile.ReadCurrent(snapshotPath, 701, started, now) == null, "previous process lifetime hidden");
            File.SetLastWriteTimeUtc(snapshotPath, now.AddSeconds(1));
            Check(AllianceEditsFile.ReadCurrent(snapshotPath, 701, started, now) == null, "future timestamp hidden");
            File.WriteAllBytes(snapshotPath, new byte[] { 0xc3, 0x28 }); File.SetLastWriteTimeUtc(snapshotPath, now);
            Check(AllianceEditsFile.ReadCurrent(snapshotPath, 701, started, now) == null, "malformed UTF8 hidden");
            File.WriteAllText(snapshotPath, new string('x', 4097)); File.SetLastWriteTimeUtc(snapshotPath, now);
            Check(AllianceEditsFile.ReadCurrent(snapshotPath, 701, started, now) == null, "oversized status hidden");
            Check(AllianceEditsFile.ReadCurrent(Path.Combine(directory, "missing.tsv"), 701, started, now) == null, "missing snapshot hidden");
            File.WriteAllText(statusPath, "Alliance edits staged; use native Confirm\n", new UTF8Encoding(false)); File.SetLastWriteTimeUtc(statusPath, now);
            Check(AllianceEditsFile.ReadStatus(statusPath, started, now) == "Alliance edits staged; use native Confirm", "fresh native status line");
            File.WriteAllText(statusPath, "bad\nextra"); File.SetLastWriteTimeUtc(statusPath, now);
            Check(AllianceEditsFile.ReadStatus(statusPath, started, now) == null, "multiline native status rejected");
            File.WriteAllText(statusPath, new string('x', 257)); File.SetLastWriteTimeUtc(statusPath, now);
            Check(AllianceEditsFile.ReadStatus(statusPath, started, now) == null, "native status length bounded");
            File.WriteAllText(statusPath, "bad\u202e"); File.SetLastWriteTimeUtc(statusPath, now);
            Check(AllianceEditsFile.ReadStatus(statusPath, started, now) == null, "nonASCII native status rejected");
            File.WriteAllText(statusPath, "Alliance dialog ready"); File.SetLastWriteTimeUtc(statusPath, now.AddSeconds(-3));
            Check(AllianceEditsFile.ReadStatus(statusPath, started, now) == null, "stale native status hidden");
        }
        finally
        {
            // This fixture created the dedicated empty directory and only files
            // inside it. No recursive deletion or game installation is involved.
            foreach (string file in Directory.GetFiles(directory)) File.Delete(file);
            Directory.Delete(directory);
        }
    }
}
