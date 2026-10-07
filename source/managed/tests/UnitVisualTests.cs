using System;
using System.Drawing;
using System.IO;
using System.Text;
using ScMultiTest;

internal static class UnitVisualTests
{
    private static int passed;
    private const string Header = "SCVIS1\t701\t219\t2\t960\t400\t1";
    private const string Row = "8193\t120\t90\t40\t20\t5120\t5120\t5120\t5120";
    private const string Valid = Header + "\n" + Row;
    private static void Check(bool value, string name)
    {
        if (!value) throw new Exception("FAILED: unit visuals: " + name);
        passed++;
    }
    private static void Reject(string text, string name)
    {
        try { UnitVisuals.Parse(text, 701); }
        catch (FormatException) { passed++; return; }
        throw new Exception("FAILED: accepted invalid unit visuals: " + name);
    }
    private sealed class ChunkedInput : MemoryStream
    {
        internal ChunkedInput(byte[] bytes) : base(bytes, false) { }
        public override long Length { get { return 0; } }
        public override int Read(byte[] buffer, int offset, int count)
        { return base.Read(buffer, offset, Math.Min(count, 7)); }
    }
    internal static int Run()
    {
        passed = 0;
        UnitVisuals value = UnitVisuals.Parse(Valid, 701);
        Check(value.Pid == 701 && value.Frame == 219 && value.Zoom == 2 && value.GameWidth == 960 && value.GameHeight == 400, "header fields");
        Check(value.Units.Length == 1 && value.Units[0].Id == 8193 && value.Units[0].Hp == 5120 && value.Units[0].Shield == 5120, "unit fields");
        Check(UnitVisuals.Parse(Valid + "\n", 701).Units.Length == 1, "LF final newline");
        Check(UnitVisuals.Parse(Valid.Replace("\n", "\r\n") + "\r\n", 701).Units.Length == 1, "CRLF rows");
        Check(UnitVisuals.Parse("SCVIS1\t701\t4294967295\t1\t1920\t800\t0", 701).Units.Length == 0, "empty visibility and maximum frame");
        Check(UnitVisuals.Parse(Header.Replace("\t2\t", "\t2e0\t") + "\n" + Row, 701).Zoom == 2, "decimal exponent zoom");
        Check(UnitVisuals.Parse(Header + "\n" + Row.Replace("\t120\t90\t", "\t-32768\t32768\t"), 701).Units[0].X == -32768, "signed coordinates bounds");
        Check(UnitVisuals.Parse(Header + "\n" + "8193\t0\t0\t192\t192\t2147483647\t1\t0\t0", 701).Units[0].MaxShield == 0, "HP upperbound and zero shields");
        Reject(null, "null"); Reject("", "empty"); Reject(new string('x', UnitVisuals.MaxBytes + 1), "oversize text");
        Reject(Valid.Replace("SCVIS1", "SCVIS2"), "protocol version");
        Reject(Valid.Replace("701", "702"), "wrong PID");
        Reject(Valid.Replace("701", "+701"), "signed PID");
        Reject(Valid.Replace("701", " 701"), "whitespace PID");
        Reject(Valid.Replace("219", "4294967296"), "frame overflow");
        Reject(Valid.Replace("\t2\t960", "\tNaN\t960"), "NaN zoom");
        Reject(Valid.Replace("\t2\t960", "\tInfinity\t960"), "infinite zoom");
        Reject(Valid.Replace("\t2\t960", "\t0\t960"), "zero zoom");
        Reject(Valid.Replace("\t2\t960", "\t16.01\t960"), "excessive zoom");
        Reject(Valid.Replace("\t960\t", "\t0\t"), "zero width");
        Reject(Header.Replace("\t1", "\t8193") + "\n" + Row, "row limit");
        Reject(Header + "\n" + Row + "\n" + Row, "extra row");
        Reject(Header.Replace("\t400\t1", "\t400\t2") + "\n" + Row + "\n" + Row, "duplicate identity");
        Reject(Header, "missing row"); Reject(Valid + "\n\n", "empty trailing row"); Reject(Valid + "\r", "bare CR");
        Reject(Header + "\n" + Row + "\textra", "extra field");
        Reject(Header + "\n" + Row.Replace("8193", "0"), "zero identity");
        Reject(Header + "\n" + Row.Replace("\t120\t", "\t32769\t"), "coordinate overflow");
        Reject(Header + "\n" + Row.Replace("\t120\t", "\t+120\t"), "positive sign coordinate");
        Reject(Header + "\n" + Row.Replace("\t40\t", "\t193\t"), "ringwidth limit");
        Reject(Header + "\n" + Row.Replace("\t20\t", "\t0\t"), "zero ringheight");
        Reject(Header + "\n8193\t0\t0\t40\t20\t0\t5120\t0\t0", "zero current HP");
        Reject(Header + "\n8193\t0\t0\t40\t20\t5120\t0\t0\t0", "zero maximum HP");
        Reject(Header + "\n8193\t0\t0\t40\t20\t5120\t5120\t-1\t5120", "negative shields");
        Reject(Valid + "\0", "NUL"); Reject(Valid + "가", "nonASCII");
        var maximum = new StringBuilder("SCVIS1\t701\t1\t2\t960\t400\t8192");
        for (int i = 1; i <= 8192; i++) maximum.Append("\n").Append(i).Append("\t0\t0\t32\t16\t256\t256\t0\t0");
        Check(UnitVisuals.Parse(maximum.ToString(), 701).Units.Length == 8192, "maximum unique owned targets");
        DateTime now = new DateTime(2026, 10, 4, 10, 0, 0, DateTimeKind.Utc), start = now.AddMinutes(-1);
        Check(UnitVisuals.IsFresh(now, start, now), "fresh current");
        Check(UnitVisuals.IsFresh(now.AddMilliseconds(-499), start, now), "fresh under halfsecond");
        Check(!UnitVisuals.IsFresh(now.AddMilliseconds(-500), start, now), "stale boundary");
        Check(!UnitVisuals.IsFresh(now.AddTicks(1), start, now), "future timestamp");
        Check(!UnitVisuals.IsFresh(start.AddTicks(-1), start, start.AddMilliseconds(1)), "prior PID lifetime");
        using (var input = new ChunkedInput(Encoding.UTF8.GetBytes(Valid)))
            Check(UnitVisuals.ReadText(input) == Valid, "partial actual reads");
        using (var input = new ChunkedInput(new byte[UnitVisuals.MaxBytes + 1]))
        {
            try { UnitVisuals.ReadText(input); throw new Exception("FAILED: accepted actual oversize visual bytes"); }
            catch (FormatException) { passed++; }
        }
        using (var input = new MemoryStream(new byte[] { 0xc3, 0x28 }))
        {
            try { UnitVisuals.ReadText(input); throw new Exception("FAILED: accepted malformed visual UTF8"); }
            catch (DecoderFallbackException) { passed++; }
        }
        float scale; int height;
        Check(value.TryProjection(new Rectangle(10, 20, 1920, 1080), out scale, out height) && scale == 2 && height == 800, "zoom projection clips gameviewport above console");
        UnitVisual unit = value.Units[0];
        Check(unit.X * scale == 240 && unit.Y * scale == 180, "world camera relative projection");
        Check(value.TryProjection(new Rectangle(0, 0, 1900, 1080), out scale, out height), "small width rounding tolerance");
        Check(!value.TryProjection(new Rectangle(0, 0, 1800, 1080), out scale, out height), "mismatched scale hidden");
        Check(!value.TryProjection(new Rectangle(0, 0, 1920, 750), out scale, out height), "viewport beyond client hidden");
        Check(!value.TryProjection(Rectangle.Empty, out scale, out height), "invalid client hidden");
        UnitVisuals small = UnitVisuals.Parse("SCVIS1\t701\t1\t0.5\t1280\t800\t1\n" + Row, 701);
        Check(small.TryProjection(new Rectangle(0, 0, 640, 480), out scale, out height) && scale == 0.5F && height == 400, "half zoom preserves world coordinates");
        Check(UnitVisuals.IsOnScreen(unit, 2, 1920, 800), "on-screen target");
        Check(UnitVisuals.IsOnScreen(new UnitVisual { X = -10, Y = 50, RingWidth = 40, RingHeight = 20 }, 2, 1920, 800), "partial left ring retained");
        Check(!UnitVisuals.IsOnScreen(new UnitVisual { X = -50, Y = 50, RingWidth = 40, RingHeight = 20 }, 2, 1920, 800), "offscreen left target culled");
        Check(!UnitVisuals.IsOnScreen(new UnitVisual { X = 50, Y = 420, RingWidth = 40, RingHeight = 20 }, 2, 1920, 800), "console target culled");
        string directory = Path.Combine(Path.GetTempPath(), "sc-visual-fixture-" + Guid.NewGuid().ToString("N"));
        string path = Path.Combine(directory, "visuals.tsv");
        Directory.CreateDirectory(directory);
        try
        {
            File.WriteAllText(path, Valid, new UTF8Encoding(false));
            DateTime stamp = File.GetLastWriteTimeUtc(path);
            Check(UnitVisuals.Read(path, 701, stamp.AddSeconds(-1), stamp) != null, "fixture file accepted fresh");
            Check(UnitVisuals.Read(path, 702, stamp.AddSeconds(-1), stamp) == null, "file wrong PID hidden");
            Check(UnitVisuals.Read(path, 701, stamp.AddSeconds(-1), stamp.AddMilliseconds(500)) == null, "file stale hidden");
            Check(UnitVisuals.Read(path, 701, stamp.AddTicks(1), stamp.AddMilliseconds(1)) == null, "file prior start hidden");
            File.WriteAllText(path, Valid + "\textra", new UTF8Encoding(false));
            stamp = File.GetLastWriteTimeUtc(path);
            Check(UnitVisuals.Read(path, 701, stamp.AddSeconds(-1), stamp) == null, "malformed file hidden");
            File.Delete(path);
            Check(UnitVisuals.Read(path, 701, stamp.AddSeconds(-1), stamp) == null, "missing file hidden");
        }
        finally { if (File.Exists(path)) File.Delete(path); Directory.Delete(directory); }
        return passed;
    }
}
