using System;
using System.Drawing;
using System.Drawing.Text;
using System.Globalization;

namespace ScMultiTest
{
    internal static class SelectionHud
    {
        internal static bool ShouldShow(Snapshot value)
        {
            return value != null && value.InGame && value.State == "READY";
        }
        internal static string NameText(Snapshot value)
        {
            if (value == null || value.DisplayCount == 0) return "<없음>";
            return value.DisplayUnitType == Snapshot.NoUnitType ? "여러 종류" : UnitNames.Get(value.DisplayUnitType);
        }
        internal static string CountText(Snapshot value)
        {
            return (value == null ? 0 : value.DisplayCount).ToString(CultureInfo.InvariantCulture);
        }
        internal static bool TryBounds(Rectangle game, out Rectangle bounds)
        {
            bounds = Rectangle.Empty;
            if (game.Width < 320 || game.Height < 200 || game.Width * 1.0 / game.Height < 1.25) return false;
            // The console follows the game client, independent of the Windows text/DPI scale.
            // 1920x1200 reference: x360..515, y995..1121 (the user's marked recess).
            // At 4:3 the recess is narrower; retain the minimap/portrait margins and fit text.
            float scale = game.Height / 480F;
            double aspect = game.Width * 1.0 / game.Height;
            float recessWidth = (float)Math.Max(30, Math.Min(62, 30 + (aspect - 4.0 / 3.0) * 120));
            bounds = new Rectangle(game.Left + (int)Math.Round(144 * scale),
                game.Top + (int)Math.Round(398 * scale),
                Math.Max(1, (int)Math.Round(recessWidth * scale)),
                Math.Max(1, (int)Math.Round(50.4F * scale)));
            if (!game.Contains(bounds)) { bounds = Rectangle.Empty; return false; }
            return true;
        }
        internal static void Draw(Graphics graphics, Snapshot value, Rectangle region)
        {
            if (!ShouldShow(value) || region.Width <= 0 || region.Height <= 0) return;
            var saved = graphics.Save();
            try
            {
                graphics.SetClip(region);
                graphics.TextRenderingHint = TextRenderingHint.SingleBitPerPixelGridFit;
                float scale = region.Height / 50.4F;
                float line = region.Height / 4F;
                Color selected = value.DisplayCount == 0 ? Color.FromArgb(191, 191, 191)
                    : value.Active ? Color.FromArgb(90, 255, 100) : Color.White;
                DrawLine(graphics, "선택된 유닛", Color.FromArgb(226, 226, 226),
                    new RectangleF(region.Left, region.Top, region.Width, line), 6.4F * scale, false);
                DrawLine(graphics, NameText(value), selected,
                    new RectangleF(region.Left, region.Top + line, region.Width, line), 7.6F * scale, true);
                DrawLine(graphics, "선택된 유닛 수", Color.FromArgb(226, 226, 226),
                    new RectangleF(region.Left, region.Top + line * 2, region.Width, line), 6.4F * scale, false);
                DrawLine(graphics, CountText(value), selected,
                    new RectangleF(region.Left, region.Top + line * 3, region.Width, line), 8F * scale, true);
            }
            finally { graphics.Restore(saved); }
        }
        private static void DrawLine(Graphics graphics, string text, Color color, RectangleF row, float pixels, bool bold)
        {
            using (var format = new StringFormat { Alignment = StringAlignment.Center,
                LineAlignment = StringAlignment.Center, Trimming = StringTrimming.EllipsisCharacter,
                FormatFlags = StringFormatFlags.NoWrap })
            {
                FontStyle style = bold ? FontStyle.Bold : FontStyle.Regular;
                float fitted = Math.Max(3F, pixels);
                // Fit headings even in a small 4:3 window. Long unit names may use an ellipsis.
                if (!bold)
                {
                    using (var probe = new Font("Malgun Gothic", fitted, style, GraphicsUnit.Pixel))
                    {
                        float measured = graphics.MeasureString(text, probe, Int32.MaxValue, StringFormat.GenericTypographic).Width;
                        if (measured > row.Width - 4) fitted = Math.Max(3F, fitted * (row.Width - 4) / measured);
                    }
                }
                using (var font = new Font("Malgun Gothic", fitted, style, GraphicsUnit.Pixel))
                using (var brush = new SolidBrush(color))
                {
                    float shadow = Math.Max(1F, (float)Math.Floor(row.Height / 18F));
                    var shadowRow = new RectangleF(row.X + shadow, row.Y + shadow, row.Width, row.Height);
                    graphics.DrawString(text, font, Brushes.Black, shadowRow, format);
                    graphics.DrawString(text, font, brush, row, format);
                }
            }
        }
    }
}