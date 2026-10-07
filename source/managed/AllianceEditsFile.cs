using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;

namespace ScMultiTest
{
    internal static class AllianceEditsFile
    {
        internal static string Serialize(AllianceEditsEventArgs edits, AllianceSnapshot current)
        {
            if (edits == null) throw Invalid();
            return SerializeChanges("SCALLYEDIT1", edits.Pid, edits.Session, edits.Generation,
                edits.Frame, edits.RequestId, edits.Changes, current, true);
        }

        internal static string SerializeApply(AllianceApplyEventArgs request, AllianceSnapshot current)
        {
            if (request == null) throw Invalid();
            return SerializeChanges("SCALLYAPPLY1", request.Pid, request.Session, request.Generation,
                request.Frame, request.RequestId, request.Changes, current, false);
        }

        private static string SerializeChanges(string marker, int pid, ulong session, ulong generation,
            uint frame, ulong requestId, IList<AllianceChange> changes, AllianceSnapshot current, bool allowEmpty)
        {
            if (current == null || !current.Open || !current.Allowed || pid <= 0 || pid != current.Pid ||
                session == 0 || generation == 0 || session != current.Session || generation != current.Generation ||
                requestId == 0 || frame > current.Frame || changes.Count > 7 || (!allowEmpty && changes.Count == 0)) throw Invalid();
            var text = new StringBuilder();
            text.AppendFormat(CultureInfo.InvariantCulture, "{0}\t{1}\t{2}\t{3}\t{4}\t{5}\t{6}\n",
                marker, pid, session, generation, frame, requestId, changes.Count);
            int previousSlot = -1;
            foreach (AllianceChange change in changes)
            {
                if (change == null) throw Invalid();
                AllianceComputerRow row = current.FindComputer(change.Slot);
                if (change.Slot < 0 || change.Slot > 7 || change.Slot <= previousSlot || row == null ||
                    !((change.ExpectedRelation == 0 && change.DesiredRelation == 1) ||
                        ((change.ExpectedRelation == 1 || change.ExpectedRelation == 2) && change.DesiredRelation == 0)) ||
                    change.ExpectedRelation != row.Relation) throw Invalid();
                previousSlot = change.Slot;
                text.AppendFormat(CultureInfo.InvariantCulture, "E\t{0}\t{1}\t{2}\n",
                    change.Slot, change.ExpectedRelation, change.DesiredRelation);
            }
            string result = text.ToString();
            if (Encoding.UTF8.GetByteCount(result) > 1024) throw Invalid();
            return result;
        }

        // Native reads one complete file. This stages checkbox edits; it never
        // invokes a sender or changes game memory. Confirm owns the commit.
        internal static void Publish(string directory, AllianceEditsEventArgs edits, AllianceSnapshot current)
        {
            string text = Serialize(edits, current);
            PublishComplete(directory, edits.Pid, "-alliance-edit.tsv", text);
        }

        // A checkbox click explicitly requests an ordinary own-player command.
        // Publication is transport acceptance; ACK plus an actual snapshot
        // must still confirm that the relationship changed.
        internal static void PublishApply(string directory, AllianceApplyEventArgs request, AllianceSnapshot current)
        {
            string text = SerializeApply(request, current);
            PublishComplete(directory, request.Pid, "-alliance-apply.tsv", text);
        }

        private static void PublishComplete(string directory, int pid, string suffix, string text)
        {
            if (String.IsNullOrWhiteSpace(directory)) throw new ArgumentException("동맹 데이터 폴더가 없습니다.", "directory");
            string root = Path.GetFullPath(directory);
            Directory.CreateDirectory(root);
            string name = "mc-" + pid.ToString(CultureInfo.InvariantCulture) + suffix;
            string target = Path.Combine(root, name);
            string temporary = Path.Combine(root, "." + name + "." + Guid.NewGuid().ToString("N") + ".tmp");
            try
            {
                byte[] bytes = new UTF8Encoding(false, true).GetBytes(text);
                using (var output = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                { output.Write(bytes, 0, bytes.Length); output.Flush(true); }
                if (File.Exists(target))
                {
                    try { File.Replace(temporary, target, null); }
                    catch (FileNotFoundException)
                    {
                        // A native consumer may remove the previous target after
                        // the Exists check. Move still publishes a complete file.
                        if (!File.Exists(temporary) || File.Exists(target)) throw;
                        File.Move(temporary, target);
                    }
                }
                else File.Move(temporary, target);
            }
            finally { if (File.Exists(temporary)) File.Delete(temporary); }
        }

        internal static AllianceSnapshot ReadCurrent(string path, int expectedPid, DateTime startedUtc, DateTime nowUtc)
        {
            try
            {
                if (String.IsNullOrEmpty(path) || !File.Exists(path) ||
                    !AllianceSnapshot.IsFresh(File.GetLastWriteTimeUtc(path), startedUtc, nowUtc)) return null;
                AllianceSnapshot value = AllianceSnapshot.Parse(SnapshotFile.Read(path), expectedPid);
                return AllianceSnapshot.IsFresh(File.GetLastWriteTimeUtc(path), startedUtc, nowUtc) ? value : null;
            }
            catch (IOException) { return null; }
            catch (UnauthorizedAccessException) { return null; }
            catch (FormatException) { return null; }
            catch (DecoderFallbackException) { return null; }
        }

        internal static string ReadStatus(string path, DateTime startedUtc, DateTime nowUtc)
        {
            try
            {
                if (!File.Exists(path) || !AllianceSnapshot.IsFresh(File.GetLastWriteTimeUtc(path), startedUtc, nowUtc)) return null;
                string text = SnapshotFile.Read(path);
                if (text.EndsWith("\r\n", StringComparison.Ordinal)) text = text.Substring(0, text.Length - 2);
                else if (text.EndsWith("\n", StringComparison.Ordinal)) text = text.Substring(0, text.Length - 1);
                if (text.Length == 0 || text.Length > 256) return null;
                foreach (char character in text) if (character < ' ' || character > '~') return null;
                return AllianceSnapshot.IsFresh(File.GetLastWriteTimeUtc(path), startedUtc, nowUtc) ? text : null;
            }
            catch (IOException) { return null; }
            catch (UnauthorizedAccessException) { return null; }
            catch (FormatException) { return null; }
            catch (DecoderFallbackException) { return null; }
        }

        // A previously staged request may outlive the launcher process. Read
        // only its sequence floor; its changes are never restored or dispatched.
        // Sequence persistence is valid throughout this game process lifetime,
        // independently of the three-second display-snapshot freshness limit.
        internal static ulong ReadSequenceFloor(string path, AllianceSnapshot current, DateTime startedUtc, DateTime nowUtc)
        { return ReadSequenceFloor(path, current, startedUtc, nowUtc, "SCALLYEDIT1", true); }

        // Apply transport has its own strict marker. A previous staged-edit
        // file cannot become an immediate request or seed its sequence floor.
        internal static ulong ReadApplySequenceFloor(string path, AllianceSnapshot current, DateTime startedUtc, DateTime nowUtc)
        { return ReadSequenceFloor(path, current, startedUtc, nowUtc, "SCALLYAPPLY1", false); }

        private static ulong ReadSequenceFloor(string path, AllianceSnapshot current, DateTime startedUtc,
            DateTime nowUtc, string marker, bool allowEmpty)
        {
            if (current == null || String.IsNullOrEmpty(path)) return 0;
            try
            {
                DateTime changedUtc = File.GetLastWriteTimeUtc(path);
                if (changedUtc < startedUtc || changedUtc > nowUtc) return 0;
                byte[] bytes = new byte[1025]; int length = 0;
                using (FileStream input = SnapshotFile.Open(path))
                {
                    while (length < bytes.Length)
                    {
                        int read = input.Read(bytes, length, bytes.Length - length);
                        if (read == 0) break;
                        length += read;
                    }
                }
                if (length == 0 || length > 1024) return 0;
                for (int i = 0; i < length; i++) if (bytes[i] > 127) return 0;
                string text = Encoding.ASCII.GetString(bytes, 0, length).Replace("\r\n", "\n");
                if (text.EndsWith("\n", StringComparison.Ordinal)) text = text.Substring(0, text.Length - 1);
                string[] lines = text.Split('\n');
                string[] header = lines[0].Split('\t');
                if (header.Length != 7 || header[0] != marker) return 0;
                ulong pid, session, generation, frame, requestId, count;
                if (!TryNumber(header[1], out pid) || !TryNumber(header[2], out session) || !TryNumber(header[3], out generation) ||
                    !TryNumber(header[4], out frame) || !TryNumber(header[5], out requestId) || !TryNumber(header[6], out count) ||
                    pid != (ulong)current.Pid || session == 0 || generation == 0 || session != current.Session ||
                    generation != current.Generation || frame > current.Frame || requestId == 0 || count > 7 ||
                    (!allowEmpty && count == 0) || lines.Length != (int)count + 1) return 0;
                int previousSlot = -1;
                for (int i = 1; i < lines.Length; i++)
                {
                    string[] row = lines[i].Split('\t');
                    ulong slot, expected, desired;
                    if (row.Length != 4 || row[0] != "E" || !TryNumber(row[1], out slot) || !TryNumber(row[2], out expected) ||
                        !TryNumber(row[3], out desired) || slot > 7 || (int)slot <= previousSlot ||
                        !((expected == 0 && desired == 1) || ((expected == 1 || expected == 2) && desired == 0))) return 0;
                    previousSlot = (int)slot;
                }
                DateTime afterUtc = File.GetLastWriteTimeUtc(path);
                return afterUtc >= startedUtc && afterUtc <= nowUtc ? requestId : 0;
            }
            catch (IOException) { return 0; }
            catch (UnauthorizedAccessException) { return 0; }
        }
        private static bool TryNumber(string text, out ulong value)
        {
            value = 0;
            if (text.Length == 0 || text.Length > 20) return false;
            foreach (char character in text) if (character < '0' || character > '9') return false;
            return UInt64.TryParse(text, NumberStyles.None, CultureInfo.InvariantCulture, out value);
        }

        private static InvalidOperationException Invalid()
        { return new InvalidOperationException("컴퓨터 동맹 상태가 변경되었습니다. 동맹창을 다시 열어 주세요."); }
    }
}
