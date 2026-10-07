using System;
using System.Diagnostics;
using System.IO;
using System.Security.Cryptography;
using System.Text;
using ScMultiTest;

internal static class OriginalFilesTests
{
    private static int passed;
    private static string testRoot;
    private static void Check(bool condition, string name)
    { if (!condition) throw new Exception("FAILED: " + name); passed++; }
    private static void Reject(Action action, string name)
    {
        try { action(); }
        catch (IOException) { passed++; return; }
        throw new Exception("FAILED (accepted): " + name);
    }
    private static bool Contains(string[] differences, string text)
    { foreach (string difference in differences) if (difference.Contains(text)) return true; return false; }
    private static string Hash(string path)
    { using (SHA256 sha = SHA256.Create()) using (FileStream stream = File.OpenRead(path)) return BitConverter.ToString(sha.ComputeHash(stream)); }
    private static void Write(string path, string data)
    { Directory.CreateDirectory(Path.GetDirectoryName(path)); File.WriteAllText(path, data, new UTF8Encoding(false)); }
    private static bool Junction(string link, string target)
    {
        // Creation only; both arguments are generated test paths under the checked temp root.
        ProcessStartInfo start = new ProcessStartInfo(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "cmd.exe"), "/c mklink /J \"" + link + "\" \"" + target + "\"");
        start.UseShellExecute = false; start.CreateNoWindow = true; start.RedirectStandardOutput = true; start.RedirectStandardError = true;
        using (Process process = Process.Start(start))
        {
            process.StandardOutput.ReadToEnd(); process.StandardError.ReadToEnd(); process.WaitForExit();
            return process.ExitCode == 0;
        }
    }
    private static void Main()
    {
        testRoot = Path.Combine(Path.GetTempPath(), "sc-originals-tests-" + Guid.NewGuid().ToString("N"));
        string fullTemp = Path.GetFullPath(Path.GetTempPath()).TrimEnd(Path.DirectorySeparatorChar) + Path.DirectorySeparatorChar;
        if (!Path.GetFullPath(testRoot).StartsWith(fullTemp, StringComparison.OrdinalIgnoreCase)) throw new Exception("Test root escaped temporary directory.");
        Directory.CreateDirectory(testRoot);
        string game = Path.Combine(testRoot, "StarCraft");
        string exe = Path.Combine(game, "x86_64", "StarCraft.exe");
        string dll = Path.Combine(game, "lib", "engine.dll");
        string upperDll = Path.Combine(game, "lib", "OPTION.DLL");
        string data = Path.Combine(game, "maps", "map.scx");
        string app = Path.Combine(testRoot, "app");
        string storage = Path.Combine(testRoot, "storage");
        string junction = Path.Combine(testRoot, "junction");
        string sourceJunction = Path.Combine(game, "junction");
        string sessionsJunction = Path.Combine(storage, "originals");
        Directory.CreateDirectory(app);
        Write(exe, "MZ fake executable fixture"); Write(dll, "MZ fake DLL fixture"); Write(upperDll, "upper-case extension"); Write(data, "unrelated map fixture");
        string beforeExe = Hash(exe), beforeDll = Hash(dll), beforeData = Hash(data);
        DateTime writeTime = File.GetLastWriteTimeUtc(exe);
        File.SetAttributes(exe, FileAttributes.ReadOnly);
        try
        {
            Reject(delegate { OriginalFiles.Capture(exe, game, storage); }, "app equal to install root");
            Reject(delegate { OriginalFiles.Capture(exe, Path.GetDirectoryName(exe), storage); }, "app under install root");
            string disallowed = Path.Combine(game, "new-storage", "sessions");
            Reject(delegate { OriginalFiles.Capture(exe, app, disallowed); }, "nonexistent storage under install root");
            Check(!Directory.Exists(Path.Combine(game, "new-storage")), "refusal creates no installation folders");
            Check(OriginalFiles.IsWithin(game, Path.Combine(game, "x86_64")), "path descendant boundary");
            Check(!OriginalFiles.IsWithin(game, game + "-other"), "sibling prefix boundary");
            Check(OriginalFiles.IsWithin(game.ToUpperInvariant(), game), "case-insensitive path boundary");
            string driveRoot = Path.GetPathRoot(testRoot);
            Check(OriginalFiles.CanonicalDirectory(driveRoot) == driveRoot, "canonical drive root keeps its separator");
            Check(OriginalFiles.IsWithin(driveRoot, testRoot), "drive root includes descendants");
            Check(OriginalFiles.IsWithin(driveRoot, driveRoot), "drive root equals itself");
            Check(!OriginalFiles.IsWithin(testRoot, driveRoot), "descendant excludes drive root");
            OriginalFiles baseline = OriginalFiles.Capture(exe, app, storage);
            Check(baseline.FileCount == 3, "only EXE and DLL backed up");
            Check(String.Equals(baseline.Root, Path.GetFullPath(game), StringComparison.OrdinalIgnoreCase), "canonical game root derived");
            Check(!OriginalFiles.IsWithin(game, baseline.BackupDirectory), "backups outside installation");
            Check(File.Exists(baseline.ManifestPath), "manifest persisted");
            Check(baseline.Verify().Length == 0, "unchanged originals and backups");
            Check(Hash(exe) == beforeExe && Hash(dll) == beforeDll && Hash(data) == beforeData, "capture preserves all source content");
            Check(File.GetLastWriteTimeUtc(exe) == writeTime && (File.GetAttributes(exe) & FileAttributes.ReadOnly) != 0, "capture preserves source timestamp and attributes");
            Check(Hash(Path.Combine(baseline.BackupDirectory, "files", "x86_64", "StarCraft.exe")) == beforeExe, "backup EXE integrity");
            Check(Hash(Path.Combine(baseline.BackupDirectory, "files", "lib", "engine.dll")) == beforeDll, "backup DLL integrity");
            Check(baseline.LastAuditPath != null && File.ReadAllText(baseline.LastAuditPath).Contains("UNCHANGED"), "external audit persisted");
            Check(Directory.GetFiles(game, "*.backup", SearchOption.AllDirectories).Length == 0, "no backup renames in installation");
            Write(data, "changed unrelated map");
            Check(baseline.Verify().Length == 0, "documented audit scope excludes maps");
            Write(dll, "modified DLL fixture");
            Check(Contains(baseline.Verify(), "원본 변경: lib"), "modified DLL detected");
            Check(File.ReadAllText(dll) == "modified DLL fixture", "verification never automatically restores originals");
            File.Delete(upperDll);
            Check(Contains(baseline.Verify(), "원본 누락: lib"), "missing DLL detected");
            Write(Path.Combine(game, "new.exe"), "new exe");
            Write(Path.Combine(game, "extra.dll"), "new dll");
            string[] changed = baseline.Verify();
            Check(Contains(changed, "원본 추가: new.exe"), "new EXE detected");
            Check(Contains(changed, "원본 추가: extra.dll"), "new DLL detected");
            Check(File.ReadAllText(baseline.LastAuditPath).Contains("DIFFERENCES"), "differences audit persisted");
            Write(Path.Combine(baseline.BackupDirectory, "files", "lib", "engine.dll"), "corrupt backup");
            Check(Contains(baseline.Verify(), "백업 변경: files"), "backup corruption detected");
            File.AppendAllText(baseline.ManifestPath, "tampering\n");
            Check(Contains(baseline.Verify(), "백업 명세 변경"), "manifest corruption detected using immutable baseline");
            Check(Hash(exe) == beforeExe, "verification still preserves read-only EXE");
            if (!Junction(junction, game)) throw new Exception("Could not create test junction; canonical path tests require it.");
            Reject(delegate { OriginalFiles.Capture(exe, junction, Path.Combine(testRoot, "unused-storage")); }, "app junction into installation rejected");
            Reject(delegate { OriginalFiles.Capture(exe, app, Path.Combine(junction, "must-not-create")); }, "storage junction nonexistent suffix rejected before write");
            Check(!Directory.Exists(Path.Combine(game, "must-not-create")), "junction refusal creates no installation folder");
            Directory.Delete(junction);
            if (!Junction(sourceJunction, app)) throw new Exception("Could not create source reparse fixture.");
            Reject(delegate { OriginalFiles.Capture(exe, app, storage); }, "installation reparse subtree refused");
            Directory.Delete(sourceJunction);
            string trapStorage = Path.Combine(testRoot, "trap-storage");
            Directory.CreateDirectory(trapStorage);
            string trapLink = Path.Combine(trapStorage, "originals");
            if (!Junction(trapLink, game)) throw new Exception("Could not create session reparse fixture.");
            try { Reject(delegate { OriginalFiles.Capture(exe, app, trapStorage); }, "originals storage junction into game refused"); }
            finally { Directory.Delete(trapLink); }
            Check(Directory.GetDirectories(game).Length == 3, "all protection checks preserve installation folder structure");
            Check(OriginalFiles.ValidateExternalDirectory(game, game + "-other") == Path.GetFullPath(game + "-other"), "outside non-existing sibling accepted");
            Reject(delegate { OriginalFiles.ValidateExternalFile(game, exe); }, "native module under game root rejected");
            string externalModule = Path.Combine(app, "module.dll");
            Write(externalModule, "fixture external module");
            Check(OriginalFiles.ValidateExternalFile(game, externalModule) == Path.GetFullPath(externalModule), "external native module accepted");
            Check(Hash(exe) == beforeExe, "final fixture EXE preserved");
            Console.WriteLine("PASS: " + passed + " original-file preservation / integrity / junction checks");
        }
        finally
        {
            // Only our known, validated temporary root is removed. Junctions are removed
            // non-recursively first so cleanup cannot enter another test subtree.
            foreach (string link in new string[] { junction, sourceJunction, Path.Combine(testRoot, "trap-storage", "originals") })
                if (Directory.Exists(link) && (File.GetAttributes(link) & FileAttributes.ReparsePoint) != 0) Directory.Delete(link);
            if (File.Exists(exe)) File.SetAttributes(exe, FileAttributes.Normal);
            string checkedRoot = Path.GetFullPath(testRoot);
            if (!checkedRoot.StartsWith(fullTemp, StringComparison.OrdinalIgnoreCase) || !Path.GetFileName(checkedRoot).StartsWith("sc-originals-tests-", StringComparison.Ordinal)) throw new Exception("Unsafe test cleanup path.");
            Directory.Delete(checkedRoot, true);
        }
    }
}
