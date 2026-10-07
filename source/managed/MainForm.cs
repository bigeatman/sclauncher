using System;
using System.Diagnostics;
using System.Drawing;
using System.Threading.Tasks;
using System.Windows.Forms;

namespace ScMultiTest
{
    internal sealed class MainForm : Form
    {
        private readonly TestMonitor monitor = new TestMonitor();
        private readonly OverlayForm overlay = new OverlayForm();
        private readonly UnitOverlayForm unitOverlay = new UnitOverlayForm();
        private readonly AllianceOverlayForm allianceOverlay = new AllianceOverlayForm { UseOwnApplyButtons = false, UseImmediateApply = true };
        private readonly ScLobbyUiLauncher.LobbyForm lobby = new ScLobbyUiLauncher.LobbyForm();
        private readonly Timer timer = new Timer();
        private readonly Label status = new Label();
        private readonly Label detail = new Label();
        private readonly Label integrity = new Label();
        private readonly TextBox history = new TextBox();
        private readonly ToolTip messageTip = new ToolTip { AutoPopDelay = 20000 };
        private readonly NotifyIcon tray = new NotifyIcon();
        private readonly Button connect = new Button();
        private readonly Button stop = new Button();
        private bool armed, closeAfterIntegrity, unitStartupWaiting;
        private DateTime waitingUntilUtc;
        private string lastAttachIdentity, lastStatus;
        private string lastAllianceStatus;
        private bool wasActive;

        internal MainForm()
        {
            Text = "SC Launcher · 로비 / 전체 제어 / 컴퓨터 동맹";
            ClientSize = new Size(720, 610);
            FormBorderStyle = FormBorderStyle.FixedSingle; MaximizeBox = false;
            StartPosition = FormStartPosition.CenterScreen;
            AutoScaleMode = AutoScaleMode.Dpi;
            BackColor = Color.FromArgb(17, 23, 31); ForeColor = Color.WhiteSmoke;
            Font = new Font("Malgun Gothic", 9F);
            var title = new Label { Text = "SC LAUNCHER", Font = new Font("Malgun Gothic", 21F), ForeColor = Color.FromArgb(255, 181, 86) };
            title.SetBounds(24, 14, 670, 38); Controls.Add(title);
            var help = new Label { Text = "백틱(`): 같은 종류 전체 제어 · 이동 / 정지 / 공격 / 생산 / 랠리\r\n동맹창: 컴퓨터 체크는 즉시 적용 · 기존 취소 버튼으로 되돌릴 수 없음" };
            help.SetBounds(26, 58, 668, 40); Controls.Add(help);
            connect.Text = "게임 연결 · 실행 대기";
            connect.SetBounds(24, 108, 672, 38);
            connect.BackColor = Color.FromArgb(150, 94, 35); connect.ForeColor = Color.WhiteSmoke;
            connect.FlatStyle = FlatStyle.Flat;
            connect.Click += async delegate { await BeginConnection(); };
            Controls.Add(connect);
            var tabs = new TabControl(); tabs.SetBounds(24, 160, 672, 380);
            var lobbyPage = new TabPage("로비 슬롯");
            var multiPage = new TabPage("유닛 전체 제어");
            lobbyPage.BackColor = multiPage.BackColor = BackColor;
            lobbyPage.ForeColor = multiPage.ForeColor = ForeColor;
            tabs.TabPages.Add(lobbyPage); tabs.TabPages.Add(multiPage); Controls.Add(tabs);
            lobby.ConfigureEmbedded(); lobbyPage.Controls.Add(lobby); lobby.Show();
            var unitHelp = new Label { Text = "같은 종류의 본인 유닛을 1개 또는 여러 개 선택한 뒤 백틱(`)을 누르세요.\r\n백틱 / Esc / 새 선택 / 창 전환 시 전체 제어가 해제됩니다.\r\n유닛명과 선택 수는 미니맵 오른쪽의 빈 공간에 표시됩니다." };
            unitHelp.SetBounds(18, 18, 620, 67); multiPage.Controls.Add(unitHelp);
            stop.Text = "전체 제어 해제"; stop.SetBounds(18, 99, 620, 36);
            stop.BackColor = Color.FromArgb(47, 58, 74); stop.ForeColor = Color.WhiteSmoke; stop.FlatStyle = FlatStyle.Flat;
            stop.Click += delegate { try { monitor.Stop(); Log("전체 제어 해제 신호를 보냈습니다."); } catch (Exception ex) { Log(ex.Message); } };
            multiPage.Controls.Add(stop);
            status.SetBounds(18, 151, 620, 40); status.ForeColor = Color.FromArgb(255, 194, 112); multiPage.Controls.Add(status);
            detail.SetBounds(18, 195, 620, 42); detail.ForeColor = Color.Silver; multiPage.Controls.Add(detail);
            history.SetBounds(18, 251, 620, 81); history.Multiline = true; history.ReadOnly = true;
            history.ScrollBars = ScrollBars.Vertical; history.BackColor = Color.FromArgb(11, 14, 20); history.ForeColor = Color.Silver;
            multiPage.Controls.Add(history);
            integrity.SetBounds(24, 558, 672, 36); integrity.ForeColor = Color.LightGreen; Controls.Add(integrity);
            tray.Icon = SystemIcons.Application; tray.Text = "SC Launcher";
            var menu = new ContextMenuStrip(); menu.Items.Add("열기", null, delegate { OpenFromTray(); }); tray.ContextMenuStrip = menu;
            tray.DoubleClick += delegate { OpenFromTray(); };
            timer.Interval = 75; timer.Tick += delegate { RefreshState(); };
            allianceOverlay.ApplyRequested += delegate(object sender, AllianceApplyEventArgs request)
            {
                try { monitor.PublishAllianceApply(request); request.AcceptedForDispatch = true; }
                catch (Exception ex) { request.Error = ex.Message; Log("컴퓨터 동맹 변경 전달 실패: " + ex.Message); }
            };
            // Retained staged transport is not used by this immediate mode.
            allianceOverlay.EditsChanged += delegate(object sender, AllianceEditsEventArgs edits)
            { edits.Error = "컴퓨터 체크는 즉시 적용 경로만 사용합니다."; };
            allianceOverlay.CancelRequested += delegate { Log("이미 보낸 컴퓨터 동맹 변경은 취소 버튼으로 되돌릴 수 없습니다. 체크를 다시 변경해 주세요."); };
        }

        protected override void OnShown(EventArgs e)
        {
            base.OnShown(e);
            Log("대상: StarCraft 1.23.10.13515 x64 · 로비 / 유닛 제어 / 컴퓨터 동맹");
            Log("빌드: 다음 게임 자동 재연결 수정 · 2026-10-07");
            Log("컴퓨터 체크는 즉시 적용하며 내 플레이어의 관계만 변경합니다. 기존 동맹창 취소 버튼은 이미 보낸 변경을 되돌리지 않습니다.");
            timer.Start(); RefreshState();
        }

        private async Task BeginConnection()
        {
            if (monitor.Connecting || closeAfterIntegrity) return;
            armed = true;
            waitingUntilUtc = DateTime.UtcNow.AddSeconds(120);
            if (lobby.HasActiveMonitor)
            {
                monitor.Connect();
                Log(monitor.Status); RefreshState();
                return;
            }
            lastAttachIdentity = null;
            TryConnectUnitModule();
            try { await lobby.StartConnection(); }
            finally { armed = false; }
        }

        internal static bool UnitStartupReady(DateTime nowUtc, DateTime startUtc, IntPtr mainWindow)
        {
            // The CEF bridge is connected early. Defer the existing native unit
            // resolver until the game window is present and initial loading settles.
            return mainWindow != IntPtr.Zero && nowUtc >= startUtc.AddSeconds(10);
        }

        private void TryConnectUnitModule()
        {
            unitStartupWaiting = false;
            if (!armed || monitor.Connecting || closeAfterIntegrity) return;
            if (monitor.Pid > 0 || monitor.IntegrityPending) return;
            if (DateTime.UtcNow > waitingUntilUtc) { armed = false; return; }
            Process[] games = Process.GetProcessesByName("StarCraft");
            try
            {
                if (games.Length != 1) return;
                DateTime processStart = games[0].StartTime.ToUniversalTime();
                if (!UnitStartupReady(DateTime.UtcNow, processStart, games[0].MainWindowHandle)) { unitStartupWaiting = true; return; }
                string identity = games[0].Id + ":" + processStart.Ticks;
                if (identity == lastAttachIdentity) return;
                lastAttachIdentity = identity;
                monitor.Connect(); Log(monitor.Status);
            }
            catch (Exception ex) { Log("게임 확인: " + ex.Message); }
            finally { foreach (Process game in games) game.Dispose(); }
        }

        private void RefreshState()
        {
            monitor.Tick(); TryConnectUnitModule();
            status.Text = unitStartupWaiting ? "스타 초기 화면 준비 대기 · 유닛 모듈 연결 전" : monitor.Status;
            integrity.Text = monitor.IntegrityStatus;
            integrity.ForeColor = monitor.IntegrityFailed ? Color.OrangeRed : Color.LightGreen;
            connect.Enabled = !monitor.Connecting && !closeAfterIntegrity && (!lobby.HasActiveMonitor || monitor.Pid > 0);
            connect.Text = lobby.HasActiveMonitor ? "현재 연결 확인 · 제어 재개" : "게임 연결 · 실행 대기";
            Snapshot value = monitor.Current;
            stop.Enabled = value != null && value.State == "READY" && value.Active;
            detail.Text = value == null ? "유닛 모듈 응답 대기" : "유닛 모듈: " + value.State + " · " + StatusText.Explain(value.Message);
            messageTip.SetToolTip(status, monitor.Status);
            if (lastStatus != monitor.Status)
            {
                if (!monitor.Status.StartsWith("선택된 유닛:", StringComparison.Ordinal)) Log(monitor.Status);
                lastStatus = monitor.Status;
            }
            bool active = value != null && value.State == "READY" && value.Active;
            if (active != wasActive) Log(active ? "전체 제어 활성화: " + value.Count + "개" : "전체 제어 해제");
            wasActive = active;
            Rectangle game;
            if (value != null && value.State == "READY" && Native.GetForegroundWindow() == monitor.Window && Native.ClientBounds(monitor.Window, out game))
            {
                unitOverlay.UpdateVisuals(active ? monitor.CurrentVisuals : null, game);
                overlay.UpdateStatus(value, game);
            }
            else { overlay.Hide(); unitOverlay.Hide(); }
            AllianceSnapshot alliance = monitor.CurrentAlliance;
            if (alliance != null) allianceOverlay.Editor.AdvanceRequestSequence(monitor.AllianceRequestSequence);
            string allianceStatus = alliance == null ? null : monitor.AllianceStatus;
            if (allianceStatus != lastAllianceStatus)
            {
                string readable = AllianceLogText(allianceStatus);
                if (readable != null) Log(readable);
                lastAllianceStatus = allianceStatus;
            }
            if (alliance == null || !alliance.Open || value == null || value.State != "READY")
                allianceOverlay.UpdateStatus(null, Rectangle.Empty);
            else if (Native.GetForegroundWindow() == monitor.Window && Native.ClientBounds(monitor.Window, out game))
                allianceOverlay.UpdateStatus(alliance, game);
            else
            {
                // Keep an in-flight checkbox request through a temporary focus
                // change, but still reconcile actual native state each tick.
                allianceOverlay.Editor.Update(alliance, DateTime.UtcNow);
                allianceOverlay.Hide();
            }
            if (closeAfterIntegrity && !monitor.IntegrityPending)
            {
                closeAfterIntegrity = false;
                if (monitor.IntegrityFailed) { OpenFromTray(); Log(monitor.IntegrityStatus); }
                else Close();
            }
        }

        private void OpenFromTray()
        {
            closeAfterIntegrity = false; tray.Visible = false;
            Show(); WindowState = FormWindowState.Normal; Activate();
        }
        private void Log(string text)
        {
            if (history.TextLength > 12000) history.Text = history.Text.Substring(history.TextLength - 6000);
            history.AppendText(DateTime.Now.ToString("HH:mm:ss") + "  " + text + Environment.NewLine);
        }
        private static string AllianceLogText(string value)
        {
            switch (value)
            {
                case "Computer alliance unsent requests cancelled": return "제어 중지 요청으로 아직 보내지 않은 컴퓨터 동맹 요청을 취소했습니다.";
                case "Native computer alliance values preserved": return "확인 명령에 컴퓨터 동맹 체크 값을 보존했습니다.";
                case "Native alliance rewrite not confirmed": return "확인 명령의 컴퓨터 동맹 값 변경을 확인하지 못했습니다.";
                case "Computer alliance applied": return "컴퓨터 동맹 관계의 실제 반영을 확인했습니다.";
                case "Computer alliance sent; awaiting actual relation": return "컴퓨터 동맹 명령 전송됨 · 실제 관계 반영 대기 중";
                case "Computer alliance not observed; check map rules": return "컴퓨터 동맹 관계 반영을 확인하지 못했습니다. 맵의 동맹 트리거를 확인해 주세요.";
                case "Computer alliance sender append not confirmed": return "컴퓨터 동맹 명령 전송을 확인하지 못했습니다. 체크를 다시 시도해 주세요.";
                case "Computer alliance state changed; retry": return "컴퓨터 동맹 상태가 바뀌어 요청을 보내지 않았습니다. 체크를 다시 시도해 주세요.";
                case "Computer alliance request expired": return "동맹창 상태가 바뀌어 요청을 보내지 않았습니다. 다시 체크해 주세요.";
                case "Computer alliance previous request pending": return "앞선 컴퓨터 동맹 요청의 실제 반영을 기다리고 있습니다.";
                case "Computer alliance sender busy; retry": return "게임 명령 버퍼가 가득 차 동맹 요청을 보내지 않았습니다. 다시 시도해 주세요.";
                case "Alliance edits staged; use native Confirm": return "컴퓨터 동맹 변경 선택됨 · 동맹창 확인을 누르면 적용됩니다.";
                case "Alliance edits stale; reopen dialog": return "컴퓨터 동맹 상태가 변경되었습니다. 동맹창을 다시 열어 주세요.";
                case "Alliance sender append not confirmed": return "컴퓨터 동맹 명령 전송을 확인하지 못했습니다. 동맹창을 다시 열어 주세요.";
                case "Alliance dialog closed; staged edits not confirmed": return "동맹창이 닫혔습니다. 선택한 컴퓨터 동맹 변경은 적용이 확인되지 않았습니다.";
                case "Alliance runtime operands unavailable":
                case "Alliance UI operands unavailable": return "컴퓨터 동맹 연결 정보를 찾지 못했습니다. 동맹 상태 로그를 확인해 주세요.";
                case "Alliance dialog layout unavailable": return "현재 동맹창의 빈 행 영역을 확인하지 못했습니다.";
                case "Alliance policy unavailable; display only": return "동맹 변경 허용 상태를 확인하지 못해 컴퓨터 목록만 표시합니다.";
                case "Alliance changes not allowed": return "현재 상태에서는 동맹 변경을 사용할 수 없습니다.";
                case "Alliance edits cancelled by stop request": return "제어 해제 요청으로 대기 중인 컴퓨터 동맹 변경도 취소했습니다.";
                case "Alliance player state unavailable": return "컴퓨터 플레이어 상태를 확인하지 못했습니다.";
                case "Alliance command appended; awaiting actual game state":
                case "Native alliance command contains edits; awaiting actual game state": return "컴퓨터 동맹 명령이 전송 버퍼에 추가되었습니다. 실제 관계 반영을 기다립니다.";
                default: return null;
            }
        }
        protected override void OnFormClosing(FormClosingEventArgs e)
        {
            armed = false; allianceOverlay.CancelPendingEdits(); allianceOverlay.UpdateStatus(null, Rectangle.Empty); monitor.SuspendControl();
            if (monitor.IntegrityPending)
            {
                e.Cancel = true; closeAfterIntegrity = true;
                overlay.Hide(); unitOverlay.Hide(); allianceOverlay.Hide(); Hide(); tray.Visible = true;
                tray.Text = "SC Launcher · 스타 종료 후 원본 검사";
                tray.ShowBalloonTip(4000, "전체 제어 중지", "스타 종료 후 원본 검사까지 알림 영역에서 대기합니다.", ToolTipIcon.Info);
                return;
            }
            lobby.StopMonitor(); base.OnFormClosing(e);
        }
        protected override void OnFormClosed(FormClosedEventArgs e)
        {
            timer.Stop(); timer.Dispose(); tray.Visible = false; tray.Dispose();
            lobby.StopMonitor(); lobby.Dispose(); overlay.Dispose(); unitOverlay.Dispose(); allianceOverlay.Dispose(); messageTip.Dispose(); base.OnFormClosed(e);
        }
    }
}
