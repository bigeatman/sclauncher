using System;
using System.Collections.Generic;
using System.Drawing;
using System.Globalization;
using System.IO;
using System.Text;

namespace ScMultiTest
{
    internal sealed class UnitVisual
    {
        internal uint Id;
        internal int X, Y, RingWidth, RingHeight, Hp, MaxHp, Shield, MaxShield;
    }

    // A bounded snapshot from the module. Contains only visible owned virtual targets.
    internal sealed class UnitVisuals
    {
        internal const int MaxBytes = 1048576;
        internal int Pid, GameWidth, GameHeight;
        internal uint Frame;
        internal float Zoom;
        internal UnitVisual[] Units;

        internal static UnitVisuals Parse(string text, int expectedPid)
        {
            if (expectedPid <= 0 || String.IsNullOrEmpty(text) || text.Length > MaxBytes) throw Invalid();
            foreach (char c in text) if (c > 126 || (c < 32 && c != '\t' && c != '\r' && c != '\n')) throw Invalid();
            if (text.EndsWith("\r\n", StringComparison.Ordinal)) text = text.Substring(0, text.Length - 2);
            else if (text.EndsWith("\n", StringComparison.Ordinal)) text = text.Substring(0, text.Length - 1);
            else if (text.EndsWith("\r", StringComparison.Ordinal)) throw Invalid();
            string[] lines = text.Split('\n');
            string[] header = TrimCr(lines[0]).Split('\t');
            if (header.Length != 7 || header[0] != "SCVIS1") throw Invalid();
            uint pid = Unsigned(header[1]), frame = Unsigned(header[2]);
            int width = Positive(header[4], 16384), height = Positive(header[5], 16384);
            uint count = Unsigned(header[6]);
            if (pid != (uint)expectedPid || count > 8192 || lines.Length != count + 1) throw Invalid();
            float zoom = Float(header[3]);
            if (zoom < 0.125F || zoom > 16F) throw Invalid();
            var units = new UnitVisual[count];
            var ids = new HashSet<uint>();
            for (int i = 0; i < units.Length; i++)
            {
                string[] fields = TrimCr(lines[i + 1]).Split('\t');
                if (fields.Length != 9) throw Invalid();
                uint id = Unsigned(fields[0]);
                if (id == 0 || !ids.Add(id)) throw Invalid();
                units[i] = new UnitVisual { Id = id, X = Coordinate(fields[1]), Y = Coordinate(fields[2]),
                    RingWidth = Positive(fields[3], 192), RingHeight = Positive(fields[4], 192),
                    Hp = Positive(fields[5], Int32.MaxValue), MaxHp = Positive(fields[6], Int32.MaxValue),
                    Shield = Nonnegative(fields[7]), MaxShield = Nonnegative(fields[8]) };
            }
            return new UnitVisuals { Pid = (int)pid, Frame = frame, Zoom = zoom,
                GameWidth = width, GameHeight = height, Units = units };
        }

        internal static bool IsFresh(DateTime changedUtc, DateTime processStartUtc, DateTime nowUtc)
        {
            return changedUtc >= processStartUtc && changedUtc <= nowUtc && (nowUtc - changedUtc).TotalMilliseconds < 500;
        }

        internal static UnitVisuals Read(string path, int expectedPid, DateTime processStartUtc, DateTime nowUtc)
        {
            try
            {
                DateTime before = File.GetLastWriteTimeUtc(path);
                if (!IsFresh(before, processStartUtc, nowUtc)) return null;
                string text;
                using (var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))
                    text = ReadText(stream);
                DateTime after = File.GetLastWriteTimeUtc(path);
                if (before != after || !IsFresh(after, processStartUtc, nowUtc)) return null;
                return Parse(text, expectedPid);
            }
            catch (IOException) { return null; }
            catch (UnauthorizedAccessException) { return null; }
            catch (FormatException) { return null; }
            catch (DecoderFallbackException) { return null; }
        }

        internal static string ReadText(Stream input)
        {
            using (var output = new MemoryStream())
            {
                byte[] buffer = new byte[8192];
                int read;
                while ((read = input.Read(buffer, 0, Math.Min(buffer.Length, MaxBytes + 1 - (int)output.Length))) != 0)
                {
                    output.Write(buffer, 0, read);
                    if (output.Length > MaxBytes) throw Invalid();
                }
                return new UTF8Encoding(false, true).GetString(output.ToArray());
            }
        }

        internal bool TryProjection(Rectangle client, out float scale, out int viewportHeight)
        {
            scale = 0; viewportHeight = 0;
            if (client.Width <= 0 || client.Height <= 0 || GameWidth <= 0 || GameHeight <= 0 || Zoom <= 0 || Single.IsNaN(Zoom) || Single.IsInfinity(Zoom)) return false;
            float width = GameWidth * Zoom, height = GameHeight * Zoom;
            float tolerance = Math.Max(4F, client.Width * 0.03F);
            if (Math.Abs(width - client.Width) > tolerance || height > client.Height + 4F || height < 1) return false;
            scale = Zoom; viewportHeight = Math.Min(client.Height, (int)Math.Ceiling(height));
            return true;
        }

        internal static bool IsOnScreen(UnitVisual unit, float scale, int width, int viewportHeight)
        {
            float x = unit.X * scale, y = unit.Y * scale;
            float halfWidth = unit.RingWidth * scale * 0.5F;
            float halfHeight = unit.RingHeight * scale * 0.5F;
            return x + halfWidth >= 0 && x - halfWidth < width && y + halfHeight + 14 * scale >= 0 && y - halfHeight < viewportHeight;
        }

        private static string TrimCr(string line)
        {
            if (line.EndsWith("\r", StringComparison.Ordinal)) line = line.Substring(0, line.Length - 1);
            if (line.IndexOf('\r') >= 0) throw Invalid();
            return line;
        }
        private static uint Unsigned(string text)
        {
            if (text.Length == 0 || text.Length > 10) throw Invalid();
            foreach (char c in text) if (c < '0' || c > '9') throw Invalid();
            uint result;
            if (!UInt32.TryParse(text, NumberStyles.None, CultureInfo.InvariantCulture, out result)) throw Invalid();
            return result;
        }
        private static int Nonnegative(string text)
        {
            uint result = Unsigned(text);
            if (result > Int32.MaxValue) throw Invalid();
            return (int)result;
        }
        private static int Positive(string text, int maximum)
        {
            int result = Nonnegative(text);
            if (result == 0 || result > maximum) throw Invalid();
            return result;
        }
        private static int Coordinate(string text)
        {
            if (text.Length == 0 || text.Length > 6) throw Invalid();
            int offset = text[0] == '-' ? 1 : 0;
            if (offset == text.Length) throw Invalid();
            for (int i = offset; i < text.Length; i++) if (text[i] < '0' || text[i] > '9') throw Invalid();
            int result;
            if (!Int32.TryParse(text, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out result) || result < -32768 || result > 32768) throw Invalid();
            return result;
        }
        private static float Float(string text)
        {
            if (text.Length == 0 || text.Length > 32) throw Invalid();
            foreach (char c in text) if ((c < '0' || c > '9') && c != '.' && c != 'e' && c != 'E' && c != '+' && c != '-') throw Invalid();
            float result;
            if (!Single.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out result) || Single.IsNaN(result) || Single.IsInfinity(result)) throw Invalid();
            return result;
        }
        private static FormatException Invalid() { return new FormatException("유효하지 않은 유닛 표시 파일입니다."); }
    }
}
