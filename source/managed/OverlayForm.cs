using System;
using System.Drawing;
using System.Windows.Forms;

namespace ScMultiTest
{
    internal sealed class OverlayForm : Form
    {
        private Snapshot snapshot;
        internal OverlayForm()
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
        protected override void WndProc(ref Message m)
        {
            if (m.Msg == 0x0084) { m.Result = new IntPtr(-1); return; }
            if (m.Msg == 0x0021) { m.Result = new IntPtr(3); return; }
            base.WndProc(ref m);
        }
        internal void UpdateStatus(Snapshot value, Rectangle game)
        {
            snapshot = value;
            Rectangle region;
            if (!SelectionHud.ShouldShow(value) || !SelectionHud.TryBounds(game, out region)) { Hide(); return; }
            Bounds = region;
            if (!Visible) Show();
            Native.SetWindowPos(Handle, new IntPtr(-1), Left, Top, Width, Height, 0x0010 | 0x0040);
            Invalidate();
        }
        protected override void OnPaint(PaintEventArgs e)
        {
            base.OnPaint(e);
            SelectionHud.Draw(e.Graphics, snapshot, ClientRectangle);
        }
    }
}