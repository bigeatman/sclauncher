using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Drawing;
using System.Globalization;

namespace ScMultiTest
{
    internal sealed class AllianceComputerRow
    {
        internal readonly int Slot, ColorIndex, Relation;
        internal AllianceComputerRow(int slot, int colorIndex, int relation)
        { Slot = slot; ColorIndex = colorIndex; Relation = relation; }
        internal bool Allied { get { return Relation != 0; } }
    }

    internal sealed class AllianceSnapshot
    {
        internal readonly int Pid, CanvasWidth, CanvasHeight;
        internal readonly ulong Session, Generation, AckRequestId;
        internal readonly uint Frame;
        internal readonly bool Open, Allowed;
        // This is an available computer-row area verified by native dialog traversal,
        // not the whole dialog or a rectangle inferred from a screenshot.
        internal readonly Rectangle RowArea;
        internal readonly ReadOnlyCollection<AllianceComputerRow> Computers;

        private AllianceSnapshot(int pid, ulong session, ulong generation, uint frame,
            bool open, bool allowed, Rectangle rowArea, int canvasWidth, int canvasHeight,
            ulong ackRequestId, IList<AllianceComputerRow> computers)
        {
            Pid = pid; Session = session; Generation = generation; Frame = frame;
            Open = open; Allowed = allowed; RowArea = rowArea;
            CanvasWidth = canvasWidth; CanvasHeight = canvasHeight; AckRequestId = ackRequestId;
            Computers = new List<AllianceComputerRow>(computers).AsReadOnly();
        }

        internal static AllianceSnapshot Parse(string text, int expectedPid)
        {
            if (expectedPid <= 0 || String.IsNullOrEmpty(text) || text.Length > 2048) throw Invalid();
            text = text.Replace("\r\n", "\n");
            if (text.EndsWith("\n", StringComparison.Ordinal)) text = text.Substring(0, text.Length - 1);
            string[] lines = text.Split('\n');
            if (lines.Length > 8) throw Invalid();
            for (int i = 0; i < lines.Length; i++)
            {
                if (lines[i].Length == 0) throw Invalid();
                foreach (char character in lines[i])
                    if (character != '\t' && (character < ' ' || character > '~')) throw Invalid();
            }
            string[] header = lines[0].Split('\t');
            if (header.Length != 14 || header[0] != "SCALLY1") throw Invalid();
            ulong pid = Number(header[1]), session = Number(header[2]), generation = Number(header[3]);
            ulong frame = Number(header[4]), ack = Number(header[13]);
            if (pid != (ulong)expectedPid || frame > UInt32.MaxValue || !Flag(header[5]) || !Flag(header[6])) throw Invalid();
            int left = Coordinate(header[7]), top = Coordinate(header[8]);
            int right = Coordinate(header[9]), bottom = Coordinate(header[10]);
            int canvasWidth = Coordinate(header[11]), canvasHeight = Coordinate(header[12]);
            bool open = header[5] == "1", allowed = header[6] == "1";
            bool emptyArea = left == 0 && top == 0 && right == 0 && bottom == 0;
            bool emptyCanvas = canvasWidth == 0 && canvasHeight == 0;
            if ((canvasWidth == 0) != (canvasHeight == 0)) throw Invalid();
            if (!emptyArea && (emptyCanvas || left >= right || top >= bottom || right > canvasWidth || bottom > canvasHeight)) throw Invalid();
            if (open && (session == 0 || generation == 0 || emptyArea || emptyCanvas)) throw Invalid();
            if (session == 0 && (open || allowed || lines.Length != 1)) throw Invalid();
            var rows = new List<AllianceComputerRow>();
            int previousSlot = -1;
            for (int i = 1; i < lines.Length; i++)
            {
                string[] fields = lines[i].Split('\t');
                if (fields.Length != 4 || fields[0] != "C") throw Invalid();
                ulong slot = Number(fields[1]), color = Number(fields[2]), relation = Number(fields[3]);
                // Only normal participating player slots are editable. Neutral slots
                // and arbitrary actor IDs must never become computer-row commands.
                // Color 16 means unknown. It is displayed as gray, never guessed
                // from the player slot or from a screenshot.
                if (slot > 7 || (int)slot <= previousSlot || color > 16 || relation > 2) throw Invalid();
                previousSlot = (int)slot;
                rows.Add(new AllianceComputerRow((int)slot, (int)color, (int)relation));
            }
            Rectangle area = emptyArea ? Rectangle.Empty : Rectangle.FromLTRB(left, top, right, bottom);
            return new AllianceSnapshot((int)pid, session, generation, (uint)frame, open, allowed,
                area, canvasWidth, canvasHeight, ack, rows);
        }

        internal static bool IsFresh(DateTime changedUtc, DateTime processStartUtc, DateTime nowUtc)
        {
            return changedUtc >= processStartUtc && changedUtc <= nowUtc && (nowUtc - changedUtc).TotalSeconds < 3;
        }
        internal AllianceComputerRow FindComputer(int slot)
        {
            foreach (AllianceComputerRow row in Computers) if (row.Slot == slot) return row;
            return null;
        }
        private static bool Flag(string text) { return text == "0" || text == "1"; }
        private static int Coordinate(string text)
        {
            ulong value = Number(text);
            if (value > 16384) throw Invalid();
            return (int)value;
        }
        private static ulong Number(string text)
        {
            if (text.Length == 0 || text.Length > 20) throw Invalid();
            foreach (char character in text) if (character < '0' || character > '9') throw Invalid();
            ulong value;
            if (!UInt64.TryParse(text, NumberStyles.None, CultureInfo.InvariantCulture, out value)) throw Invalid();
            return value;
        }
        private static FormatException Invalid() { return new FormatException("유효하지 않은 컴퓨터 동맹 상태 파일입니다."); }
    }

    internal sealed class AllianceChange
    {
        internal readonly int Slot, ExpectedRelation, DesiredRelation;
        internal AllianceChange(int slot, int expectedRelation, int desiredRelation)
        { Slot = slot; ExpectedRelation = expectedRelation; DesiredRelation = desiredRelation; }
    }

    internal sealed class AllianceApplyEventArgs : EventArgs
    {
        internal readonly int Pid;
        internal readonly ulong Session, Generation, RequestId;
        internal readonly uint Frame;
        internal readonly ReadOnlyCollection<AllianceChange> Changes;
        // The host sets this only after the request has actually been dispatched.
        internal bool AcceptedForDispatch { get; set; }
        internal string Error { get; set; }
        internal AllianceApplyEventArgs(AllianceSnapshot snapshot, ulong requestId, IList<AllianceChange> changes)
        {
            Pid = snapshot.Pid; Session = snapshot.Session; Generation = snapshot.Generation;
            Frame = snapshot.Frame; RequestId = requestId;
            Changes = new List<AllianceChange>(changes).AsReadOnly();
        }
    }

    internal sealed class AllianceEditsEventArgs : EventArgs
    {
        internal readonly int Pid;
        internal readonly ulong Session, Generation, RequestId;
        internal readonly uint Frame;
        internal readonly ReadOnlyCollection<AllianceChange> Changes;
        // Publishing stages UI edits only. Native Confirm remains the sole
        // command trigger; publishing is not proof that a relation changed.
        internal bool Published { get; set; }
        internal string Error { get; set; }
        internal AllianceEditsEventArgs(AllianceSnapshot snapshot, ulong requestId, IList<AllianceChange> changes)
        {
            Pid = snapshot.Pid; Session = snapshot.Session; Generation = snapshot.Generation;
            Frame = snapshot.Frame; RequestId = requestId;
            Changes = new List<AllianceChange>(changes).AsReadOnly();
        }
    }

    // Pending checkboxes are UI state. Actual computer relations always come from
    // a new native snapshot; neither a click nor a dispatch ACK proves application.
    internal sealed class AllianceEditor
    {
        private readonly Dictionary<int, int> desired = new Dictionary<int, int>();
        private ulong nextRequestId;
        private AllianceApplyEventArgs inFlight;
        private AllianceEditsEventArgs publishedEdits;
        private DateTime submittedUtc;
        internal AllianceSnapshot Current { get; private set; }
        internal string Notice { get; private set; }
        internal bool Busy { get { return inFlight != null; } }
        internal bool Dirty { get { return desired.Count != 0; } }
        internal ReadOnlyCollection<AllianceChange> PendingChanges { get { return Changes().AsReadOnly(); } }
        internal void AdvanceRequestSequence(ulong minimum)
        {
            if (minimum > nextRequestId) nextRequestId = minimum;
            if (nextRequestId == UInt64.MaxValue) Notice = "동맹 요청 식별자를 더 만들 수 없습니다. 게임을 재시작해 주세요.";
        }

        internal void Update(AllianceSnapshot value, DateTime nowUtc)
        {
            AllianceSnapshot old = Current;
            bool same = SameContext(old, value);
            if (same && (unchecked((int)(value.Frame - old.Frame)) < 0 || value.AckRequestId < old.AckRequestId)) return;
            bool sameRoster = same && SameRoster(old, value);
            Current = value;
            if (value != null && value.AckRequestId > nextRequestId) nextRequestId = value.AckRequestId;
            if (value == null || !value.Open || !same || !sameRoster)
            {
                desired.Clear(); inFlight = null; publishedEdits = null; Notice = null;
                return;
            }
            if (!value.Allowed)
            {
                desired.Clear(); inFlight = null; publishedEdits = null; Notice = "이 게임에서는 동맹을 변경할 수 없습니다.";
                return;
            }
            if (inFlight != null)
            {
                bool matches = true;
                foreach (AllianceChange change in inFlight.Changes)
                {
                    AllianceComputerRow row = value.FindComputer(change.Slot);
                    if (row == null || row.Relation != change.DesiredRelation) matches = false;
                }
                if (value.AckRequestId >= inFlight.RequestId && matches)
                { desired.Clear(); inFlight = null; Notice = null; }
                else if (nowUtc >= submittedUtc && (nowUtc - submittedUtc).TotalSeconds >= 5)
                { desired.Clear(); inFlight = null; Notice = "동맹 설정 반영을 확인하지 못했습니다."; }
                return;
            }
            if (publishedEdits != null && value.AckRequestId >= publishedEdits.RequestId && Matches(value, publishedEdits.Changes))
            {
                desired.Clear(); publishedEdits = null; Notice = null;
                return;
            }
            if (Dirty && RelationsChanged(old, value))
            { desired.Clear(); publishedEdits = null; Notice = "동맹 상태가 변경되어 선택을 초기화했습니다."; }
        }

        internal bool Checked(int slot)
        {
            AllianceComputerRow row = Current == null ? null : Current.FindComputer(slot);
            int relation;
            return row != null && (desired.TryGetValue(slot, out relation) ? relation != 0 : row.Allied);
        }
        internal bool Toggle(int slot)
        {
            if (Current == null || !Current.Open || !Current.Allowed || Busy) return false;
            if (nextRequestId == UInt64.MaxValue)
            { Notice = "동맹 요청 식별자를 더 만들 수 없습니다. 게임을 재시작해 주세요."; return false; }
            AllianceComputerRow row = Current.FindComputer(slot);
            if (row == null) return false;
            int relation = Checked(slot) ? 0 : row.Allied ? row.Relation : 1;
            if (relation == row.Relation) desired.Remove(slot); else desired[slot] = relation;
            Notice = null;
            return true;
        }
        internal AllianceApplyEventArgs PrepareRequest()
        {
            if (Current == null || !Current.Open || !Current.Allowed || Busy || !Dirty || nextRequestId == UInt64.MaxValue) return null;
            return new AllianceApplyEventArgs(Current, ++nextRequestId, Changes());
        }
        internal AllianceEditsEventArgs PrepareEdits()
        {
            if (Current == null || !Current.Open || !Current.Allowed || Busy || nextRequestId == UInt64.MaxValue) return null;
            // An empty diff explicitly clears native pending computer edits.
            return new AllianceEditsEventArgs(Current, ++nextRequestId, Changes());
        }
        internal void FinishPublish(AllianceEditsEventArgs edits)
        {
            if (edits == null || Current == null || edits.Pid != Current.Pid || edits.Session != Current.Session || edits.Generation != Current.Generation) return;
            if (!edits.Published)
            {
                // Native may still hold the previous complete publication. A
                // failed newer write must not leave different checkbox choices
                // on screen for native Confirm to commit unexpectedly.
                desired.Clear();
                if (publishedEdits != null && publishedEdits.Pid == Current.Pid &&
                    publishedEdits.Session == Current.Session && publishedEdits.Generation == Current.Generation)
                    foreach (AllianceChange change in publishedEdits.Changes) desired[change.Slot] = change.DesiredRelation;
                Notice = "컴퓨터 동맹 선택 전달에 실패하여 선택을 되돌렸습니다.";
                return;
            }
            publishedEdits = edits;
            Notice = Dirty ? "확인을 누르면 적용됩니다." : null;
        }
        internal void FinishDispatch(AllianceApplyEventArgs request, DateTime nowUtc)
        {
            if (request == null || Current == null || request.Pid != Current.Pid || request.Session != Current.Session || request.Generation != Current.Generation) return;
            if (!request.AcceptedForDispatch)
            { Notice = String.IsNullOrWhiteSpace(request.Error) ? "동맹 변경 요청을 보내지 못했습니다." : request.Error; return; }
            inFlight = request; submittedUtc = nowUtc; Notice = "동맹 설정 반영 중";
        }
        internal bool CancelPending()
        {
            if (Busy) return false;
            desired.Clear(); publishedEdits = null; Notice = null;
            return true;
        }
        private List<AllianceChange> Changes()
        {
            var changes = new List<AllianceChange>();
            if (Current != null)
                foreach (AllianceComputerRow row in Current.Computers)
                {
                    int relation;
                    if (desired.TryGetValue(row.Slot, out relation)) changes.Add(new AllianceChange(row.Slot, row.Relation, relation));
                }
            return changes;
        }
        private static bool Matches(AllianceSnapshot snapshot, IList<AllianceChange> changes)
        {
            foreach (AllianceChange change in changes)
            {
                AllianceComputerRow row = snapshot.FindComputer(change.Slot);
                if (row == null || row.Relation != change.DesiredRelation) return false;
            }
            return true;
        }
        internal void CompleteRequest(ulong session, ulong generation, ulong requestId, bool accepted, string error)
        {
            if (inFlight == null || inFlight.Session != session || inFlight.Generation != generation || inFlight.RequestId != requestId) return;
            // Acceptance only permits waiting for a fresh relation snapshot.
            if (!accepted)
            { desired.Clear(); inFlight = null; Notice = String.IsNullOrWhiteSpace(error) ? "동맹 변경이 적용되지 않았습니다." : error; }
        }
        private static bool SameContext(AllianceSnapshot a, AllianceSnapshot b)
        {
            return a != null && b != null && a.Pid == b.Pid && a.Session == b.Session && a.Generation == b.Generation;
        }
        private static bool SameRoster(AllianceSnapshot a, AllianceSnapshot b)
        {
            if (a.Computers.Count != b.Computers.Count) return false;
            for (int i = 0; i < a.Computers.Count; i++)
                if (a.Computers[i].Slot != b.Computers[i].Slot || a.Computers[i].ColorIndex != b.Computers[i].ColorIndex) return false;
            return true;
        }
        private static bool RelationsChanged(AllianceSnapshot a, AllianceSnapshot b)
        {
            for (int i = 0; i < a.Computers.Count; i++)
                if (a.Computers[i].Relation != b.Computers[i].Relation) return true;
            return false;
        }
    }
}
