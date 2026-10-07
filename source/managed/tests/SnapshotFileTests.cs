using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using ScMultiTest;

internal static class SnapshotFileTests
{
    private const string Ready = "SCMULTI1\t701\t0\t0\t0\t0\tREADY\tReady";
    private const string Active = "SCMULTI1\t701\t1\t15\t37\t24\tREADY\tActive";
    private static int passed;
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true, SetLastError = true)]
    private static extern bool MoveFileExW(string existing, string replacement, uint flags);
    private static void Check(bool value, string name)
    {
        if (!value) throw new Exception("FAILED: " + name);
        passed++;
    }
    private static void RejectSize(Stream input, string name)
    {
        try { SnapshotFile.ReadText(input); }
        catch (FormatException) { passed++; return; }
        throw new Exception("FAILED (accepted oversize file): " + name);
    }
    private sealed class ChunkedInput : MemoryStream
    {
        internal ChunkedInput(byte[] bytes) : base(bytes, false) { }
        // Model an untrusted or stale size: bounding must depend on bytes read.
        public override long Length { get { return 0; } }
        public override int Read(byte[] buffer, int offset, int count)
        { return base.Read(buffer, offset, Math.Min(count, 7)); }
    }
    internal static int Run()
    {
        passed = 0;
        using (var input = new ChunkedInput(Encoding.UTF8.GetBytes(Active)))
            Check(SnapshotFile.ReadText(input) == Active, "partial reads preserve snapshot");
        using (var input = new MemoryStream(new byte[4096]))
            Check(SnapshotFile.ReadText(input).Length == 4096, "reader byte limit boundary");
        using (var input = new ChunkedInput(new byte[4097]))
            RejectSize(input, "actual reads bounded despite stale length");
        using (var input = new MemoryStream(new byte[] { 0xc3, 0x28 }))
        {
            try { SnapshotFile.ReadText(input); throw new Exception("FAILED: accepted malformed UTF8"); }
            catch (DecoderFallbackException) { passed++; }
        }
        using (var input = new MemoryStream(Encoding.UTF8.GetPreamble()))
            Check(SnapshotFile.ReadText(input) == "", "UTF8 BOM handling retained");

        string directory = Path.Combine(Path.GetTempPath(), "sc-snapshot-fixture-" + Guid.NewGuid().ToString("N"));
        string path = Path.Combine(directory, "state.tsv"), replacement = Path.Combine(directory, "state.tmp");
        string retired = Path.Combine(directory, "retired.tsv");
        Directory.CreateDirectory(directory);
        try
        {
            File.WriteAllText(path, Ready, new UTF8Encoding(false));
            Check(SnapshotFile.Read(path) == Ready, "read status fixture");
            using (FileStream oldReader = SnapshotFile.Open(path))
            {
                // Verify delete sharing with a rename of the open source. MoveFileEx
                // can still deny replacement of an open destination on this system.
                if (!MoveFileExW(path, retired, 0)) throw new Win32Exception(Marshal.GetLastWin32Error());
                passed++;
                File.WriteAllText(path, Active, new UTF8Encoding(false));
                Check(SnapshotFile.ReadText(oldReader) == Ready, "open handle retains complete renamed snapshot");
                Check(SnapshotFile.Read(path) == Active, "new open sees complete published snapshot");
            }
            File.WriteAllText(replacement, Ready, new UTF8Encoding(false));
            if (!MoveFileExW(replacement, path, 1)) throw new Win32Exception(Marshal.GetLastWin32Error());
            Check(SnapshotFile.Read(path) == Ready, "replacement succeeds after reader disposal");
            using (FileStream input = SnapshotFile.Open(path))
            {
                long before = input.Length;
                using (FileStream writer = new FileStream(path, FileMode.Append, FileAccess.Write, FileShare.ReadWrite | FileShare.Delete))
                {
                    byte[] extension = new byte[4097];
                    writer.Write(extension, 0, extension.Length);
                }
                Check(before < 4096, "fixture was small when reader opened");
                RejectSize(input, "file growth after open is bounded");
            }
        }
        finally
        {
            File.Delete(replacement);
            File.Delete(path);
            File.Delete(retired);
            Directory.Delete(directory);
        }
        return passed;
    }
}