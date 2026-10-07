using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Text;
using System.Windows.Forms;

namespace ScMultiTest
{
    internal sealed class AllianceOverlayLayout
    {
        internal readonly Rectangle Bounds, Rows, Apply, Cancel, Status;
        internal readonly int RowHeight, VisibleRows;
        private readonly int[] rowBoundaries;
        private AllianceOverlayLayout(Rectangle bounds, Rectangle rows, Rectangle apply,
            Rectangle cancel, Rectangle status, int rowHeight, int visibleRows, int[] rowBoundaries = null)
        { Bounds = bounds; Rows = rows; Apply = apply; Cancel = cancel; Status = status; RowHeight = rowHeight; VisibleRows = visibleRows; this.rowBoundaries = rowBoundaries; }

        internal static bool TryCreate(AllianceSnapshot value, Rectangle game, out AllianceOverlayLayout layout)
        { return TryCreate(value, game, false, out layout); }
        internal static bool TryCreate(AllianceSnapshot value, Rectangle game, bool ownApplyButtons, out AllianceOverlayLayout layout)
        {
            layout = null;
            if (value == null || !value.Open || value.Computers.Count == 0 || value.RowArea.IsEmpty ||
                value.CanvasWidth == 0 || value.CanvasHeight == 0 || game.Width < 320 || game.Height < 200) return false;
            double sx = game.Width * 1.0 / value.CanvasWidth, sy = game.Height * 1.0 / value.CanvasHeight;
            int left = game.Left + (int)Math.Round(value.RowArea.Left * sx);
            int top = game.Top + (int)Math.Round(value.RowArea.Top * sy);
            int right = game.Left + (int)Math.Round(value.RowArea.Right * sx);
            int bottom = game.Top + (int)Math.Round(value.RowArea.Bottom * sy);
            Rectangle bounds = Rectangle.FromLTRB(left, top, right, bottom);
            if (!game.Contains(bounds)) return false;
            if (!ownApplyButtons)
            {
                // Native AllyFltr human rows are 26 canvas units high. The
                // verified row area excludes the victory and Confirm controls.
                int nativeRowHeight = (int)Math.Round(26 * sy);
                // Capacity belongs to the verified logical UI area. Repeatedly
                // adding a rounded pixel pitch loses rows at fractional scales
                // (for example 26 * 1.75 = 45.5); round each boundary instead.
                int nativeCapacity = value.RowArea.Height / 26;
                if (bounds.Width < 140 || nativeCapacity < 1) return false;
                if (nativeRowHeight < 20) return false;
                int count = Math.Min(value.Computers.Count, nativeCapacity);
                int rowsBottom = (int)Math.Round(count * 26 * sy);
                int spare = bounds.Height - rowsBottom;
                // A scrolling page needs clickable arrows, even when the game
                // does not route the wheel to an inactive overlay window.
                if (value.Computers.Count > nativeCapacity && spare < 12)
                {
                    if (count <= 1) return false;
                    count--; rowsBottom = (int)Math.Round(count * 26 * sy);
                    spare = bounds.Height - rowsBottom;
                }
                if (rowsBottom <= 0 || rowsBottom > bounds.Height) return false;
                int[] boundaries = new int[count + 1];
                for (int i = 0; i <= count; i++) boundaries[i] = (int)Math.Round(i * 26 * sy);
                var nativeRows = new Rectangle(0, 0, bounds.Width, rowsBottom);
                // A 1–3 pixel remainder cannot show an error or instruction.
                // Such messages are delivered by the launcher's normal log.
                var nativeStatus = spare >= 12 ? new Rectangle(0, nativeRows.Bottom, bounds.Width, spare) : Rectangle.Empty;
                layout = new AllianceOverlayLayout(bounds, nativeRows, Rectangle.Empty, Rectangle.Empty, nativeStatus, nativeRowHeight, count, boundaries);
                return true;
            }
            int rowHeight = Math.Max(20, (int)Math.Round(18 * sy));
            int footerHeight = Math.Max(24, (int)Math.Round(20 * sy));
            int statusHeight = Math.Max(12, (int)Math.Round(8 * sy));
            int padding = Math.Max(3, (int)Math.Round(2 * sy));
            int capacity = (bounds.Height - footerHeight - statusHeight - padding * 3) / rowHeight;
            if (bounds.Width < 140 || capacity < 1) return false;
            int visibleRows = Math.Min(value.Computers.Count, capacity);
            var rows = new Rectangle(padding, padding, bounds.Width - padding * 2, rowHeight * visibleRows);
            int footerTop = bounds.Height - footerHeight - padding;
            int buttonWidth = (bounds.Width - padding * 3) / 2;
            var apply = new Rectangle(padding, footerTop, buttonWidth, footerHeight);
            var cancel = new Rectangle(padding * 2 + buttonWidth, footerTop, buttonWidth, footerHeight);
            var status = new Rectangle(padding, footerTop - statusHeight - padding, bounds.Width - padding * 2, statusHeight);
            layout = new AllianceOverlayLayout(bounds, rows, apply, cancel, status, rowHeight, visibleRows);
            return true;
        }

        internal Rectangle Row(int visibleIndex)
        {
            if (rowBoundaries != null)
                return new Rectangle(Rows.Left, Rows.Top + rowBoundaries[visibleIndex], Rows.Width,
                    rowBoundaries[visibleIndex + 1] - rowBoundaries[visibleIndex]);
            return new Rectangle(Rows.Left, Rows.Top + visibleIndex * RowHeight, Rows.Width, RowHeight);
        }
        internal int RowIndexAt(Point location)
        {
            if (!Rows.Contains(location)) return -1;
            for (int i = 0; i < VisibleRows; i++) if (Row(i).Contains(location)) return i;
            return -1;
        }
    }

    // This panel is interactive but never activates over the game. It is shown
    // only in an area positively identified by the native module. The launcher
    // uses immediate checkbox requests; legacy staging remains an explicit mode.
    internal sealed class AllianceOverlayForm : Form
    {
        private readonly AllianceEditor editor = new AllianceEditor();
        private AllianceOverlayLayout layout;
        private int firstRow;
        private static readonly Color[] PlayerColors = {
            Color.FromArgb(244, 4, 4), Color.FromArgb(12, 72, 204), Color.FromArgb(44, 180, 148), Color.FromArgb(132, 64, 156),
            Color.FromArgb(248, 140, 20), Color.FromArgb(112, 48, 20), Color.FromArgb(204, 224, 208), Color.FromArgb(252, 252, 56),
            Color.FromArgb(8, 128, 8), Color.FromArgb(252, 252, 124), Color.FromArgb(188, 188, 188), Color.FromArgb(116, 164, 252),
            Color.FromArgb(148, 144, 252), Color.FromArgb(252, 176, 172), Color.FromArgb(24, 24, 24), Color.FromArgb(228, 228, 228),
            Color.Gray
        };

        internal event EventHandler<AllianceApplyEventArgs> ApplyRequested;
        internal event EventHandler CancelRequested;
        internal event EventHandler<AllianceEditsEventArgs> EditsChanged;
        internal bool UseOwnApplyButtons { get; set; }
        internal bool UseImmediateApply { get; set; }
        internal AllianceEditor Editor { get { return editor; } }
        internal AllianceOverlayForm()
        {
            FormBorderStyle = FormBorderStyle.None; ShowInTaskbar = false; TopMost = true;
            AutoScaleMode = AutoScaleMode.None; StartPosition = FormStartPosition.Manual;
            BackColor = Color.FromArgb(10, 18, 42); DoubleBuffered = true;
            TabStop = false;
        }
        protected override bool ShowWithoutActivation { get { return true; } }
        protected override CreateParams CreateParams
        {
            get { var value = base.CreateParams; value.ExStyle |= 0x00000080 | 0x08000000; return value; }
        }
        protected override void WndProc(ref Message m)
        {
            // MA_NOACTIVATE permits mouse clicks without transferring focus.
            if (m.Msg == 0x0021) { m.Result = new IntPtr(3); return; }
            base.WndProc(ref m);
        }

        internal void UpdateStatus(AllianceSnapshot value, Rectangle game)
        { UpdateStatus(value, game, DateTime.UtcNow); }
        internal void UpdateStatus(AllianceSnapshot value, Rectangle game, DateTime nowUtc)
        {
            AllianceSnapshot previous = editor.Current;
            bool wasDirty = editor.Dirty;
            editor.Update(value, nowUtc);
            value = editor.Current;
            if (previous == null || value == null || previous.Pid != value.Pid ||
                previous.Session != value.Session || previous.Generation != value.Generation) firstRow = 0;
            AllianceOverlayLayout next;
            if (!AllianceOverlayLayout.TryCreate(value, game, UseOwnApplyButtons, out next)) { layout = null; Hide(); return; }
            layout = next; firstRow = Math.Max(0, Math.Min(firstRow, value.Computers.Count - layout.VisibleRows));
            Bounds = layout.Bounds;
            if (!Visible) Show();
            Native.SetWindowPos(Handle, new IntPtr(-1), Left, Top, Width, Height, 0x0010 | 0x0040);
            if (wasDirty && !editor.Dirty && !UseOwnApplyButtons && !UseImmediateApply) PublishEdits();
            Invalidate();
        }
        internal void CompleteRequest(ulong session, ulong generation, ulong requestId, bool accepted, string error)
        { editor.CompleteRequest(session, generation, requestId, accepted, error); Invalidate(); }
        internal void CancelPendingEdits()
        {
            if (!editor.CancelPending()) return;
            if (!UseOwnApplyButtons && !UseImmediateApply) PublishEdits();
            Invalidate();
        }
        private void PublishEdits()
        {
            AllianceEditsEventArgs edits = editor.PrepareEdits();
            if (edits == null) return;
            EventHandler<AllianceEditsEventArgs> handler = EditsChanged;
            if (handler != null) handler(this, edits);
            editor.FinishPublish(edits);
        }

        // Pure click-to-dispatch operation so delivery and acknowledgement can
        // be checked without creating an overlay or touching a game window.
        internal static bool DispatchImmediateToggle(AllianceEditor state, int slot,
            Action<AllianceApplyEventArgs> dispatch, DateTime nowUtc)
        {
            if (state == null || !state.Toggle(slot)) return false;
            AllianceApplyEventArgs request = state.PrepareRequest();
            if (request == null) return false;
            try { if (dispatch != null) dispatch(request); }
            catch (Exception ex) { request.Error = ex.Message; request.AcceptedForDispatch = false; }
            // A delivery failure has not committed anything. Restore the
            // observed relationship, then retain the explicit failure notice.
            if (!request.AcceptedForDispatch) state.CancelPending();
            state.FinishDispatch(request, nowUtc);
            return true;
        }
        private void ApplyImmediateToggle(int slot)
        {
            DispatchImmediateToggle(editor, slot, delegate(AllianceApplyEventArgs request)
            {
                EventHandler<AllianceApplyEventArgs> handler = ApplyRequested;
                if (handler != null) handler(this, request);
            }, DateTime.UtcNow);
        }

        protected override void OnMouseDown(MouseEventArgs e)
        {
            base.OnMouseDown(e);
            if (e.Button != MouseButtons.Left || layout == null || editor.Current == null) return;
            if (layout.VisibleRows < editor.Current.Computers.Count && layout.Status.Contains(e.Location) &&
                (e.X < layout.Status.Left + layout.Status.Height || e.X >= layout.Status.Right - layout.Status.Height))
            {
                int direction = e.X < layout.Status.Left + layout.Status.Height ? -1 : 1;
                firstRow = Math.Max(0, Math.Min(editor.Current.Computers.Count - layout.VisibleRows, firstRow + direction));
            }
            else if (layout.Rows.Contains(e.Location))
            {
                int visibleIndex = layout.RowIndexAt(e.Location);
                int index = firstRow + visibleIndex;
                if (visibleIndex >= 0 && visibleIndex < layout.VisibleRows && index < editor.Current.Computers.Count)
                {
                    int slot = editor.Current.Computers[index].Slot;
                    if (UseImmediateApply) ApplyImmediateToggle(slot);
                    else if (editor.Toggle(slot) && !UseOwnApplyButtons) PublishEdits();
                }
            }
            else if (layout.Apply.Contains(e.Location))
            {
                AllianceApplyEventArgs request = editor.PrepareRequest();
                if (request != null)
                {
                    EventHandler<AllianceApplyEventArgs> handler = ApplyRequested;
                    if (handler != null) handler(this, request);
                    editor.FinishDispatch(request, DateTime.UtcNow);
                }
            }
            else if (layout.Cancel.Contains(e.Location) && editor.CancelPending())
            {
                EventHandler handler = CancelRequested;
                if (handler != null) handler(this, EventArgs.Empty);
            }
            Invalidate();
        }
        protected override void OnMouseWheel(MouseEventArgs e)
        {
            base.OnMouseWheel(e);
            if (layout == null || editor.Current == null || e.Delta == 0) return;
            firstRow = Math.Max(0, Math.Min(editor.Current.Computers.Count - layout.VisibleRows, firstRow + (e.Delta < 0 ? 1 : -1)));
            Invalidate();
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            base.OnPaint(e);
            if (layout == null || editor.Current == null) return;
            Graphics graphics = e.Graphics;
            graphics.TextRenderingHint = TextRenderingHint.SingleBitPerPixelGridFit;
            graphics.SmoothingMode = SmoothingMode.None;
            using (var border = new Pen(Color.FromArgb(61, 71, 157)))
                graphics.DrawRectangle(border, 0, 0, Math.Max(0, Width - 1), Math.Max(0, Height - 1));
            for (int i = 0; i < layout.VisibleRows; i++)
            {
                AllianceComputerRow computer = editor.Current.Computers[firstRow + i];
                Rectangle row = layout.Row(i);
                int icon = Math.Max(10, Math.Min(layout.RowHeight - 8, layout.RowHeight * 3 / 5));
                int inset = Math.Max(3, layout.RowHeight / 8);
                Rectangle swatch = new Rectangle(row.Left + inset, row.Top + (row.Height - icon) / 2, icon, icon);
                using (var brush = new SolidBrush(PlayerColors[computer.ColorIndex])) graphics.FillRectangle(brush, swatch);
                Rectangle check = new Rectangle(row.Right - inset - icon, swatch.Top, icon, icon);
                using (var pen = new Pen(editor.Current.Allowed ? Color.FromArgb(116, 123, 223) : Color.FromArgb(78, 84, 104), Math.Max(1, icon / 12)))
                    graphics.DrawRectangle(pen, check);
                if (editor.Checked(computer.Slot))
                {
                    using (var pen = new Pen(editor.Current.Allowed ? Color.Yellow : Color.Gray, Math.Max(2, icon / 7)))
                    {
                        graphics.DrawLine(pen, check.Left + 2, check.Top + 2, check.Right - 2, check.Bottom - 2);
                        graphics.DrawLine(pen, check.Left + 2, check.Bottom - 2, check.Right - 2, check.Top + 2);
                    }
                }
                Rectangle text = Rectangle.FromLTRB(swatch.Right + inset * 2, row.Top, check.Left - inset * 2, row.Bottom);
                DrawText(graphics, "컴퓨터 (P" + (computer.Slot + 1) + ")", text,
                    editor.Current.Allowed ? Color.WhiteSmoke : Color.Silver, Math.Max(10, layout.RowHeight * 0.56F), false);
            }
            if (UseOwnApplyButtons)
            {
                bool enabled = editor.Current.Allowed && !editor.Busy;
                DrawButton(graphics, layout.Apply, "컴퓨터 적용", enabled && editor.Dirty);
                DrawButton(graphics, layout.Cancel, "컴퓨터 취소", !editor.Busy);
            }
            string status = !editor.Current.Allowed ? "동맹 변경 불가" : editor.Notice;
            if (String.IsNullOrEmpty(status) && editor.Dirty) status = "변경 대기";
            if (String.IsNullOrEmpty(status) && layout.VisibleRows < editor.Current.Computers.Count)
                status = (firstRow + 1) + "–" + (firstRow + layout.VisibleRows) + "/" + editor.Current.Computers.Count + " · 휠로 이동";
            Rectangle statusRegion = layout.Status;
            if (!statusRegion.IsEmpty && layout.VisibleRows < editor.Current.Computers.Count)
            {
                int arrowWidth = layout.Status.Height;
                DrawText(graphics, "↑", new Rectangle(statusRegion.Left, statusRegion.Top, arrowWidth, statusRegion.Height),
                    firstRow > 0 ? Color.Yellow : Color.Gray, Math.Max(10, statusRegion.Height * 0.85F), true);
                DrawText(graphics, "↓", new Rectangle(statusRegion.Right - arrowWidth, statusRegion.Top, arrowWidth, statusRegion.Height),
                    firstRow + layout.VisibleRows < editor.Current.Computers.Count ? Color.Yellow : Color.Gray,
                    Math.Max(10, statusRegion.Height * 0.85F), true);
                statusRegion = Rectangle.FromLTRB(statusRegion.Left + arrowWidth, statusRegion.Top, statusRegion.Right - arrowWidth, statusRegion.Bottom);
            }
            if (!statusRegion.IsEmpty)
                DrawText(graphics, status ?? String.Empty, statusRegion, Color.FromArgb(185, 193, 214),
                    Math.Max(9, Math.Min(layout.RowHeight * 0.45F, layout.Status.Height * 0.8F)), true);
        }
        private static void DrawButton(Graphics graphics, Rectangle rectangle, string text, bool enabled)
        {
            using (var pen = new Pen(enabled ? Color.FromArgb(81, 94, 232) : Color.FromArgb(54, 61, 85), 1))
                graphics.DrawRectangle(pen, rectangle);
            DrawText(graphics, text, rectangle, enabled ? Color.Yellow : Color.Gray, Math.Max(10, rectangle.Height * 0.55F), true);
        }
        private static void DrawText(Graphics graphics, string text, Rectangle rectangle, Color color, float size, bool center)
        {
            using (var font = new Font("Malgun Gothic", size, FontStyle.Regular, GraphicsUnit.Pixel))
            using (var brush = new SolidBrush(color))
            using (var format = new StringFormat { Alignment = center ? StringAlignment.Center : StringAlignment.Near,
                LineAlignment = StringAlignment.Center, Trimming = StringTrimming.EllipsisCharacter, FormatFlags = StringFormatFlags.NoWrap })
                graphics.DrawString(text, font, brush, rectangle, format);
        }
    }
}
