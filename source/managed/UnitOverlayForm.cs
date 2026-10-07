using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Windows.Forms;

namespace ScMultiTest
{
    internal sealed class UnitOverlayForm : Form
    {
        private UnitVisuals visuals;
        private float scale;
        private int viewportHeight;
        internal UnitOverlayForm()
        {
            FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; TopMost = true;
            BackColor = Color.Magenta; TransparencyKey = Color.Magenta;
            AutoScaleMode = AutoScaleMode.None;
            StartPosition = FormStartPosition.Manual; DoubleBuffered = true;
        }
        protected override bool ShowWithoutActivation { get { return true; } }
        protected override CreateParams CreateParams
        {
            get { var value = base.CreateParams; value.ExStyle |= 0x00000080 | 0x08000000 | 0x00000020; return value; }
        }
        protected override void WndProc(ref Message message)
        {
            if (message.Msg == 0x0084) { message.Result = new IntPtr(-1); return; }
            if (message.Msg == 0x0021) { message.Result = new IntPtr(3); return; }
            base.WndProc(ref message);
        }
        internal void UpdateVisuals(UnitVisuals value, Rectangle game)
        {
            float nextScale; int nextHeight;
            if (value == null || value.Units.Length == 0 || !value.TryProjection(game, out nextScale, out nextHeight))
            { visuals = null; Hide(); return; }
            visuals = value; scale = nextScale; viewportHeight = nextHeight;
            Bounds = game;
            if (!Visible) Show();
            Native.SetWindowPos(Handle, new IntPtr(-1), Left, Top, Width, Height, 0x0010 | 0x0040);
            Invalidate();
        }
        protected override void OnPaint(PaintEventArgs e)
        {
            base.OnPaint(e);
            if (visuals == null) return;
            // Color-key transparency and crisp lines avoid tinted alpha edges.
            e.Graphics.SmoothingMode = SmoothingMode.None;
            e.Graphics.SetClip(new Rectangle(0, 0, Width, viewportHeight));
            using (var pen = new Pen(Color.FromArgb(33, 213, 56), Math.Max(1F, scale)))
            {
                foreach (UnitVisual unit in visuals.Units)
                {
                    if (!UnitVisuals.IsOnScreen(unit, scale, Width, viewportHeight)) continue;
                    float x = unit.X * scale, y = unit.Y * scale;
                    float ringWidth = unit.RingWidth * scale, ringHeight = unit.RingHeight * scale;
                    e.Graphics.DrawEllipse(pen, x - ringWidth * 0.5F, y - ringHeight * 0.5F, ringWidth, ringHeight);
                    float barWidth = Math.Max(24F, Math.Min(80F, unit.RingWidth)) * scale;
                    float barHeight = Math.Max(2F, 3F * scale), gap = Math.Max(1F, scale);
                    float top = y + ringHeight * 0.5F + 4F * scale;
                    if (unit.MaxShield > 0)
                    { DrawBar(e.Graphics, x, top, barWidth, barHeight, unit.Shield, unit.MaxShield, Brushes.RoyalBlue); top += barHeight + gap; }
                    Brush health = unit.Hp * 1.0 / unit.MaxHp > 0.66 ? Brushes.LimeGreen : (unit.Hp * 1.0 / unit.MaxHp > 0.33 ? Brushes.Gold : Brushes.Red);
                    DrawBar(e.Graphics, x, top, barWidth, barHeight, unit.Hp, unit.MaxHp, health);
                }
            }
        }
        private static void DrawBar(Graphics graphics, float center, float y, float width, float height, int current, int maximum, Brush color)
        {
            float left = center - width * 0.5F;
            graphics.FillRectangle(Brushes.Black, left - 1, y - 1, width + 2, height + 2);
            double ratio = Math.Max(0, Math.Min(1, current * 1.0 / maximum));
            if (ratio > 0) graphics.FillRectangle(color, left, y, (float)(width * ratio), height);
            int segments = Math.Max(1, (int)(width / 8));
            for (int i = 1; i < segments; i++)
                graphics.DrawLine(Pens.Black, left + width * i / segments, y, left + width * i / segments, y + height);
        }
    }
}
