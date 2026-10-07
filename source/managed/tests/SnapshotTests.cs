using System;
using ScMultiTest;

internal static class SnapshotTests
{
    private static int passed;
    private const int Pid = 701;
    private const string Ready = "SCMULTI1\t701\t0\t0\t0\t0\tREADY\tReady";
    private const string Active = "SCMULTI1\t701\t1\t15\t37\t24\tREADY\tActive";
    private static void Check(bool condition, string name)
    {
        if (!condition) throw new Exception("FAILED: " + name);
        passed++;
    }
    private static void Reject(string text, string name)
    {
        try { Snapshot.Parse(text, Pid); }
        catch (FormatException) { passed++; return; }
        throw new Exception("FAILED (accepted invalid input): " + name);
    }
    private static void Main()
    {
        try { Run(); }
        catch (Exception ex)
        {
            Console.Error.WriteLine("FAIL: " + ex.GetType().Name + ": " + ex.Message);
            Environment.ExitCode = 1;
        }
    }
    private static void Run()
    {
        Snapshot value = Snapshot.Parse(Active, Pid);
        Check(value.Active && value.Count == 15 && value.UnitType == 37 && value.AppendedBatches == 24 && value.State == "READY", "active values");
        Check(!Snapshot.Parse(Ready, Pid).Active, "inactive ready");
        Check(Snapshot.Parse(Ready + "\n", Pid).State == "READY", "LF terminator");
        Check(Snapshot.Parse(Ready + "\r\n", Pid).State == "READY", "CRLF terminator");
        Check(Snapshot.Parse(Ready.Replace("READY", "WAITING"), Pid).State == "WAITING", "waiting");
        Check(Snapshot.Parse(Ready.Replace("READY", "FAILED"), Pid).State == "FAILED", "failure");
        Check(Snapshot.Parse("SCMULTI1\t701\t1\t8192\t65535\t18446744073709551615\tREADY\tMax", Pid).AppendedBatches == UInt64.MaxValue, "numeric maxima");
        Reject(null, "null"); Reject("", "empty"); Reject(new string('a', 1025), "oversize");
        Reject(Ready.Replace("SCMULTI1", "SCMULTI2"), "unknown version");
        Reject(Ready.Replace("701", "702"), "wrong PID");
        Reject(Ready.Replace("701", "0"), "zero PID");
        Reject(Ready.Replace("701", "2147483648"), "oversize PID");
        Reject(Ready.Replace("701", "+701"), "signed number");
        Reject(Ready.Replace("701", " 701"), "whitespace number");
        Reject(Ready.Replace("701", "７０１"), "nonascii number");
        Reject("SCMULTI1\t701\t2\t0\t0\t0\tREADY\tReady", "invalid active flag");
        Reject("SCMULTI1\t701\t1\t8193\t0\t0\tREADY\tActive", "count limit");
        Reject("SCMULTI1\t701\t1\t15\t65536\t0\tREADY\tActive", "unit limit");
        Reject("SCMULTI1\t701\t1\t15\t0\t18446744073709551616\tREADY\tActive", "u64 overflow");
        Reject("SCMULTI1\t701\t1\t-15\t0\t0\tREADY\tActive", "negative count");
        Reject("SCMULTI1\t701\t1\t0\t0\t0\tREADY\tActive", "active with no units");
        Reject("SCMULTI1\t701\t0\t15\t0\t0\tREADY\tActive", "inactive phantom count");
        Reject(Active.Replace("READY", "WAITING"), "active while waiting");
        Reject(Active.Replace("READY", "FAILED"), "active while failed");
        Reject(Active.Replace("READY", "SUCCESS"), "unknown state");
        Reject(Ready.Substring(0, Ready.LastIndexOf('\t')), "missing field");
        Reject(Ready + "\textra", "extra field");
        Reject(Ready + "\n\n", "multiple trailing lines");
        Reject(Ready + "\r", "bare CR");
        Reject(Ready + "\nextra", "appended row");
        Reject(Ready + "\0", "nul");
        Reject(Ready.Replace("\tReady", "\t"), "empty message");
        Reject(Ready.Replace("\tReady", "\t   "), "blank message");
        Reject(Ready.Replace("\tReady", "\t" + new string('a', 513)), "long message");
        Reject(Ready + "\u202e", "bidi message");
        Reject(Ready + "\u007f", "DEL message");
        Reject(Ready + "가", "nonASCII message");
        DateTime now = new DateTime(2026, 9, 27, 10, 0, 0, DateTimeKind.Utc);
        DateTime start = now.AddMinutes(-2);
        Check(Snapshot.IsFresh(now, start, now), "current timestamp");
        Check(Snapshot.IsFresh(now.AddMilliseconds(-2999), start, now), "fresh below 3seconds");
        Check(!Snapshot.IsFresh(now.AddSeconds(-3), start, now), "stale at boundary");
        Check(!Snapshot.IsFresh(now.AddSeconds(-10), start, now), "stale value");
        Check(!Snapshot.IsFresh(now.AddTicks(1), start, now), "future timestamp");
        Check(!Snapshot.IsFresh(start.AddTicks(-1), start, start.AddSeconds(1)), "prior PID lifetime");
        Check(Snapshot.IsFresh(start, start, start.AddSeconds(1)), "same startup time");
        try { Snapshot.Parse(Ready, 0); throw new Exception("accepted invalid expected PID"); }
        catch (FormatException) { passed++; }
        passed += SnapshotFileTests.Run();
        passed += ConnectionRetryTests.Run();
        passed += MenuWaitTests.Run();
        passed += UnitVisualTests.Run();
        passed += SelectionHudTests.Run();
        Console.WriteLine("PASS: " + passed + " snapshot / freshness / file / connection retry / menu wait / unit visual / selection HUD checks");
    }
}
