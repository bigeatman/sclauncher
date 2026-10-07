using System;
using System.Globalization;

namespace ScMultiTest
{
    internal sealed class Snapshot
    {
        internal const int NoUnitType = 65535;
        internal int Pid, Count, UnitType;
        internal bool Active;
        // SCMULTI2 adds the ordinary selection when virtual control is inactive.
        internal int ProtocolVersion, DisplayCount, DisplayUnitType;
        internal bool InGame;
        // Wire commandsSent counts batches appended to the outgoing buffer, not accepted orders or moves.
        internal ulong AppendedBatches;
        internal string State, Message;

        internal static Snapshot Parse(string text, int expectedPid)
        {
            if (expectedPid <= 0 || String.IsNullOrEmpty(text) || text.Length > 1024) throw Invalid();
            if (text.EndsWith("\r\n", StringComparison.Ordinal)) text = text.Substring(0, text.Length - 2);
            else if (text.EndsWith("\n", StringComparison.Ordinal)) text = text.Substring(0, text.Length - 1);
            string[] fields = text.Split('\t');
            bool v2 = fields.Length == 11 && fields[0] == "SCMULTI2";
            if (!v2 && (fields.Length != 8 || fields[0] != "SCMULTI1")) throw Invalid();
            ulong pid = Number(fields[1]), count = Number(fields[3]), unit = Number(fields[4]), commands = Number(fields[5]);
            if (pid != (ulong)expectedPid || !Flag(fields[2]) || count > 8192 || unit > 65535) throw Invalid();
            string state = fields[6], message = fields[7];
            if (state != "READY" && state != "WAITING" && state != "FAILED") throw Invalid();
            if (String.IsNullOrWhiteSpace(message) || message.Length > 512) throw Invalid();
            foreach (char c in message) if (c < ' ' || c > '~') throw Invalid();
            bool active = fields[2] == "1";
            if ((active && (state != "READY" || count == 0)) || (!active && count != 0)) throw Invalid();
            bool inGame = active;
            ulong displayCount = active ? count : 0;
            ulong displayUnit = active ? unit : NoUnitType;
            if (v2)
            {
                if (!Flag(fields[8])) throw Invalid();
                inGame = fields[8] == "1";
                displayCount = Number(fields[9]); displayUnit = Number(fields[10]);
                if (displayCount > 8192 || displayUnit > 65535) throw Invalid();
                if ((displayCount == 0 && displayUnit != NoUnitType) || (!inGame && displayCount != 0)) throw Invalid();
                if (active && (!inGame || displayCount != count || displayUnit != unit || displayUnit == NoUnitType)) throw Invalid();
            }
            return new Snapshot { Pid = (int)pid, Active = active, Count = (int)count, UnitType = (int)unit,
                AppendedBatches = commands, State = state, Message = message,
                ProtocolVersion = v2 ? 2 : 1, InGame = inGame,
                DisplayCount = (int)displayCount, DisplayUnitType = (int)displayUnit };
        }
        internal static bool IsFresh(DateTime changedUtc, DateTime processStartUtc, DateTime nowUtc)
        {
            return changedUtc >= processStartUtc && changedUtc <= nowUtc && (nowUtc - changedUtc).TotalSeconds < 3;
        }
        private static bool Flag(string text) { return text == "0" || text == "1"; }
        private static ulong Number(string text)
        {
            if (text.Length == 0 || text.Length > 20) throw Invalid();
            foreach (char c in text) if (c < '0' || c > '9') throw Invalid();
            ulong result;
            if (!UInt64.TryParse(text, NumberStyles.None, CultureInfo.InvariantCulture, out result)) throw Invalid();
            return result;
        }
        private static FormatException Invalid() { return new FormatException("유효하지 않은 테스트 상태 파일입니다."); }
    }
}