using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace ScMultiTest
{
    // Installation paths are only ever opened for read. Backups and reports belong to
    // a canonical, external session directory; this class has no restore operation.
    internal sealed class OriginalFiles
    {
        private const long MaximumBytes = 2L * 1024 * 1024 * 1024;
        private const int MaximumFiles = 10000;
        private sealed class Entry
        {
            internal readonly string RelativePath;
            internal readonly long Length;
            internal readonly string Hash;
            internal Entry(string path, long length, string hash)
            { RelativePath = path; Length = length; Hash = hash; }
        }
        private readonly Entry[] entries;
        private readonly string manifestHash;
        public string Root { get; private set; }
        public string BackupDirectory { get; private set; }
        public string ManifestPath { get; private set; }
        public string LastAuditPath { get; private set; }
        public int FileCount { get { return entries.Length; } }

        private OriginalFiles(string root, string backup, Entry[] values, string manifest, string hash)
        { Root = root; BackupDirectory = backup; entries = values; ManifestPath = manifest; manifestHash = hash; }

        public static OriginalFiles Capture(string executablePath, string appDirectory, string storageDirectory)
        {
            if (String.IsNullOrWhiteSpace(executablePath) || String.IsNullOrWhiteSpace(appDirectory) || String.IsNullOrWhiteSpace(storageDirectory))
                throw new ArgumentException("게임, 도구, 저장 폴더 경로가 필요합니다.");
            executablePath = Path.GetFullPath(executablePath);
            if ((File.GetAttributes(executablePath) & FileAttributes.ReparsePoint) != 0)
                throw new IOException("게임 실행 파일이 재분석 지점입니다. 연결하지 않습니다.");
            string root = ValidateExternalPaths(executablePath, appDirectory, storageDirectory);
            string storage = CanonicalDirectory(storageDirectory);
            Directory.CreateDirectory(storage);
            storage = CanonicalExistingPath(storage, true);
            RequireOutside(root, storage, "보관");
            string sessions = Path.Combine(storage, "originals");
            Directory.CreateDirectory(sessions);
            sessions = CanonicalExistingPath(sessions, true);
            RequireOutside(root, sessions, "백업");
            string backup = Path.Combine(sessions, DateTime.UtcNow.ToString("yyyyMMddTHHmmssfffZ", CultureInfo.InvariantCulture) + "-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(backup);
            backup = CanonicalExistingPath(backup, true);
            RequireOutside(root, backup, "백업");

            List<string> files = ExecutableFiles(root);
            if (files.Count == 0) throw new IOException("게임 실행 파일을 찾지 못했습니다.");
            List<Entry> values = new List<Entry>();
            long totalBytes = 0;
            foreach (string path in files)
            {
                string relative = Relative(root, path);
                string destination = Path.Combine(backup, "files", relative);
                string destinationDirectory = Path.GetDirectoryName(destination);
                Directory.CreateDirectory(destinationDirectory);
                string safeDirectory = CanonicalExistingPath(destinationDirectory, true);
                RequireOutside(root, safeDirectory, "백업");
                if (!IsWithin(backup, safeDirectory)) throw new IOException("백업 폴더 경로가 세션 외부로 변경됐습니다.");
                EnsurePlainPath(root, relative);
                long length;
                string hash;
                // FileShare.Read prevents concurrent writes or deletion during the copy.
                using (FileStream original = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read))
                {
                    length = original.Length;
                    totalBytes = checked(totalBytes + length);
                    if (totalBytes > MaximumBytes) throw new IOException("원본 백업이 2 GiB 제한을 초과했습니다. 연결하지 않습니다.");
                    using (FileStream copy = new FileStream(destination, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                    {
                        original.CopyTo(copy, 65536);
                        copy.Flush(true);
                    }
                    original.Position = 0;
                    hash = Hash(original);
                    if (original.Length != length || !String.Equals(hash, HashFile(destination), StringComparison.Ordinal))
                        throw new IOException("백업 검증에 실패했습니다: " + relative);
                }
                values.Add(new Entry(relative, length, hash));
            }
            string manifest = Path.Combine(backup, "manifest.tsv");
            StringBuilder text = new StringBuilder("SCMULTIORIGINALS1\n");
            text.Append("ROOT\t").Append(Base64(root)).Append('\n');
            text.Append("UTC\t").Append(DateTime.UtcNow.ToString("o", CultureInfo.InvariantCulture)).Append('\n');
            foreach (Entry entry in values)
                text.Append("FILE\t").Append(Base64(entry.RelativePath)).Append('\t').Append(entry.Length.ToString(CultureInfo.InvariantCulture)).Append('\t').Append(entry.Hash).Append('\n');
            WriteNew(manifest, text.ToString());
            OriginalFiles result = new OriginalFiles(root, backup, values.ToArray(), manifest, HashFile(manifest));
            string[] initial = result.Verify();
            if (initial.Length != 0) throw new IOException("원본/백업 검증에 실패했습니다. 게임에는 연결하지 않습니다.\n" + String.Join("\n", initial));
            return result;
        }

        public string[] Verify()
        {
            List<string> differences = new List<string>();
            Dictionary<string, Entry> expected = new Dictionary<string, Entry>(StringComparer.OrdinalIgnoreCase);
            foreach (Entry entry in entries) expected.Add(entry.RelativePath, entry);
            foreach (Entry entry in entries)
                Compare(Root, entry.RelativePath, entry.Length, entry.Hash, "원본", differences);
            try
            {
                foreach (string path in ExecutableFiles(Root))
                {
                    string relative = Relative(Root, path);
                    if (!expected.ContainsKey(relative)) differences.Add("원본 추가: " + relative);
                }
            }
            catch (Exception error) { differences.Add("원본 목록 확인 실패: " + error.Message); }
            foreach (Entry entry in entries)
                Compare(BackupDirectory, Path.Combine("files", entry.RelativePath), entry.Length, entry.Hash, "백업", differences);
            try
            {
                EnsurePlainPath(BackupDirectory, "manifest.tsv");
                if (!String.Equals(manifestHash, HashFile(ManifestPath), StringComparison.Ordinal)) differences.Add("백업 명세 변경: manifest.tsv");
            }
            catch (Exception error) { differences.Add("백업 명세 확인 실패: " + error.Message); }
            LastAuditPath = null;
            try
            {
                string reportDirectory = CanonicalExistingPath(BackupDirectory, true);
                RequireOutside(Root, reportDirectory, "감사 보고서");
                if (!String.Equals(reportDirectory, BackupDirectory, StringComparison.OrdinalIgnoreCase)) throw new IOException("세션 폴더가 변경됐습니다.");
                string report = Path.Combine(reportDirectory, "audit-" + DateTime.UtcNow.ToString("yyyyMMddTHHmmssfffZ", CultureInfo.InvariantCulture) + "-" + Guid.NewGuid().ToString("N") + ".txt");
                string body = "SC MultiTest original files audit\r\nUTC: " + DateTime.UtcNow.ToString("o", CultureInfo.InvariantCulture) + "\r\nRoot: " + Root + "\r\nFiles: " + entries.Length.ToString(CultureInfo.InvariantCulture) + "\r\nResult: " + (differences.Count == 0 ? "UNCHANGED" : "DIFFERENCES") + "\r\n" + String.Join("\r\n", differences) + "\r\n";
                WriteNew(report, body);
                LastAuditPath = report;
            }
            catch (Exception error) { differences.Add("감사 보고서 저장 실패: " + error.Message); }
            return differences.ToArray();
        }

        internal static string ValidateExternalPaths(string executablePath, string appDirectory, string storageDirectory)
        {
            if ((File.GetAttributes(executablePath) & FileAttributes.ReparsePoint) != 0)
                throw new IOException("게임 실행 파일이 재분석 지점입니다. 연결하지 않습니다.");
            string canonicalExecutable = CanonicalExistingPath(executablePath, false);
            string executableDirectory = Path.GetDirectoryName(canonicalExecutable);
            string directoryName = Path.GetFileName(executableDirectory);
            string root = (String.Equals(directoryName, "x86_64", StringComparison.OrdinalIgnoreCase) || String.Equals(directoryName, "x86", StringComparison.OrdinalIgnoreCase))
                ? Path.GetDirectoryName(executableDirectory) : executableDirectory;
            root = CanonicalExistingPath(root, true);
            ValidateExternalDirectory(root, appDirectory);
            ValidateExternalDirectory(root, storageDirectory);
            return root;
        }
        internal static string ValidateExternalFile(string gameRoot, string path)
        {
            FileAttributes attributes = File.GetAttributes(path);
            if ((attributes & (FileAttributes.ReparsePoint | FileAttributes.Directory)) != 0)
                throw new IOException("외부 모듈 파일이 일반 파일이 아닙니다. 연결하지 않습니다.");
            string root = CanonicalExistingPath(gameRoot, true);
            string external = CanonicalExistingPath(path, false);
            RequireOutside(root, external, "모듈");
            return external;
        }        internal static string ValidateExternalDirectory(string gameRoot, string path)
        {
            string root = CanonicalExistingPath(gameRoot, true);
            string external = CanonicalDirectory(path);
            RequireOutside(root, external, "도구/보관");
            return external;
        }
        // Also used by the attach gate. Resolve before checking; lexical paths alone
        // permit a directory junction to disguise a location inside the installation.
        internal static string CanonicalDirectory(string path)
        {
            path = Path.GetFullPath(path);
            if (Directory.Exists(path)) return CanonicalExistingPath(path, true);
            List<string> suffix = new List<string>();
            string current = path;
            while (!Directory.Exists(current))
            {
                if (File.Exists(current)) throw new IOException("폴더 대신 파일이 있습니다: " + current);
                string name = Path.GetFileName(current.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar));
                string parent = Path.GetDirectoryName(current);
                if (String.IsNullOrEmpty(parent) || String.IsNullOrEmpty(name)) throw new DirectoryNotFoundException(path);
                suffix.Add(name);
                current = parent;
            }
            current = CanonicalExistingPath(current, true);
            for (int i = suffix.Count - 1; i >= 0; i--) current = Path.Combine(current, suffix[i]);
            return Path.GetFullPath(current);
        }

        internal static bool IsWithin(string root, string path)
        {
            root = NormalizePath(root);
            path = NormalizePath(path);
            string prefix = root.EndsWith(Path.DirectorySeparatorChar.ToString(), StringComparison.Ordinal) ? root : root + Path.DirectorySeparatorChar;
            return String.Equals(root, path, StringComparison.OrdinalIgnoreCase) || path.StartsWith(prefix, StringComparison.OrdinalIgnoreCase);
        }
        private static string NormalizePath(string path)
        {
            string full = Path.GetFullPath(path);
            string volumeRoot = Path.GetPathRoot(full);
            return full.Length <= volumeRoot.Length ? full : full.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
        }        private static void RequireOutside(string root, string path, string description)
        { if (IsWithin(root, path)) throw new IOException(description + " 폴더가 게임 설치 폴더 내부입니다. 원본 보호를 위해 연결하지 않습니다."); }
        private static string Relative(string root, string path)
        {
            if (!IsWithin(root, path) || String.Equals(root, path, StringComparison.OrdinalIgnoreCase)) throw new IOException("원본 파일 경로가 설치 범위를 벗어났습니다.");
            string prefix = root.EndsWith(Path.DirectorySeparatorChar.ToString(), StringComparison.Ordinal) ? root : root + Path.DirectorySeparatorChar;
            return path.Substring(prefix.Length);
        }
        private static List<string> ExecutableFiles(string root)
        {
            List<string> result = new List<string>();
            Stack<string> pending = new Stack<string>();
            pending.Push(root);
            int directories = 0;
            while (pending.Count != 0)
            {
                string directory = pending.Pop();
                if (++directories > MaximumFiles) throw new IOException("설치 폴더 수 제한을 초과했습니다.");
                if ((File.GetAttributes(directory) & FileAttributes.ReparsePoint) != 0) throw new IOException("재분석 폴더를 발견했습니다: " + directory);
                foreach (string child in Directory.GetDirectories(directory))
                {
                    if ((File.GetAttributes(child) & FileAttributes.ReparsePoint) != 0) throw new IOException("재분석 폴더를 발견했습니다: " + child);
                    pending.Push(child);
                }
                foreach (string file in Directory.GetFiles(directory))
                {
                    if ((File.GetAttributes(file) & FileAttributes.ReparsePoint) != 0) throw new IOException("재분석 파일을 발견했습니다: " + file);
                    string extension = Path.GetExtension(file);
                    if (String.Equals(extension, ".exe", StringComparison.OrdinalIgnoreCase) || String.Equals(extension, ".dll", StringComparison.OrdinalIgnoreCase))
                    {
                        result.Add(file);
                        if (result.Count > MaximumFiles) throw new IOException("원본 실행 파일 수 제한을 초과했습니다.");
                    }
                }
            }
            result.Sort(StringComparer.OrdinalIgnoreCase);
            return result;
        }
        private static void EnsurePlainPath(string root, string relative)
        {
            if (Path.IsPathRooted(relative)) throw new IOException("상대 경로가 아닙니다.");
            string current = root;
            if ((File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0) throw new IOException("폴더가 재분석 지점으로 변경됐습니다.");
            foreach (string part in relative.Split(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar))
            {
                if (part == ".." || part == "." || part.Length == 0) throw new IOException("잘못된 상대 경로입니다.");
                current = Path.Combine(current, part);
                if ((File.GetAttributes(current) & FileAttributes.ReparsePoint) != 0) throw new IOException("경로가 재분석 지점입니다.");
            }
        }
        private static void Compare(string root, string relative, long length, string hash, string label, List<string> differences)
        {
            string path = Path.Combine(root, relative);
            try
            {
                EnsurePlainPath(root, relative);
                using (FileStream stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read))
                {
                    if (stream.Length != length || !String.Equals(Hash(stream), hash, StringComparison.Ordinal)) differences.Add(label + " 변경: " + relative);
                }
            }
            catch (FileNotFoundException) { differences.Add(label + " 누락: " + relative); }
            catch (DirectoryNotFoundException) { differences.Add(label + " 누락: " + relative); }
            catch (Exception error) { differences.Add(label + " 확인 실패: " + relative + " (" + error.Message + ")"); }
        }
        private static string Hash(Stream stream)
        { using (SHA256 algorithm = SHA256.Create()) return BitConverter.ToString(algorithm.ComputeHash(stream)).Replace("-", ""); }
        private static string HashFile(string path)
        { using (FileStream stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read)) return Hash(stream); }
        private static string Base64(string text) { return Convert.ToBase64String(Encoding.UTF8.GetBytes(text)); }
        private static void WriteNew(string path, string text)
        { using (FileStream stream = new FileStream(path, FileMode.CreateNew, FileAccess.Write, FileShare.Read)) using (StreamWriter writer = new StreamWriter(stream, new UTF8Encoding(false))) writer.Write(text); }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern SafeFileHandle CreateFileW(string path, uint access, uint share, IntPtr security, uint creation, uint flags, IntPtr template);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern uint GetFinalPathNameByHandleW(SafeFileHandle file, StringBuilder path, uint length, uint flags);
        private static string CanonicalExistingPath(string path, bool directory)
        {
            using (SafeFileHandle handle = CreateFileW(Path.GetFullPath(path), 0, 1 | 2 | 4, IntPtr.Zero, 3, directory ? 0x02000000u : 0u, IntPtr.Zero))
            {
                if (handle.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error(), "경로 확인 실패: " + path);
                StringBuilder result = new StringBuilder(512);
                uint length = GetFinalPathNameByHandleW(handle, result, (uint)result.Capacity, 0);
                if (length == 0) throw new Win32Exception(Marshal.GetLastWin32Error(), "최종 경로 확인 실패: " + path);
                if (length >= result.Capacity)
                {
                    result = new StringBuilder(checked((int)length + 1));
                    length = GetFinalPathNameByHandleW(handle, result, (uint)result.Capacity, 0);
                    if (length == 0 || length >= result.Capacity) throw new IOException("최종 경로가 너무 깁니다.");
                }
                string value = result.ToString();
                if (value.StartsWith(@"\\?\UNC\", StringComparison.OrdinalIgnoreCase)) value = @"\\" + value.Substring(8);
                else if (value.StartsWith(@"\\?\", StringComparison.Ordinal)) value = value.Substring(4);
                return NormalizePath(value);
            }
        }
    }
}
