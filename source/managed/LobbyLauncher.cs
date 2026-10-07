using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Globalization;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using System.Windows.Forms;

namespace ScLobbyUiLauncher {
 internal sealed class LobbyForm : Form {
  readonly Button connect = new Button();
  readonly TextBox history = new TextBox();
  readonly Label status = new Label();
  readonly Timer timer = new Timer();
  readonly string folder = AppDomain.CurrentDomain.BaseDirectory;
  readonly string logFolder = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "SCMultiTestLobby");
  readonly Dictionary<int, int> browsers = new Dictionary<int, int>();
  Process ownedMonitor;
  string lastLog, failure;
  bool connecting, closing, startupWaiting;
  int helperId, initialStage;
  DateTime connectionStartedUtc;

  internal LobbyForm() {
   Text = "SC MultiTest · 방장 로비 메뉴";
   ClientSize = new Size(650, 380);
   StartPosition = FormStartPosition.CenterScreen;
   FormBorderStyle = FormBorderStyle.FixedSingle;
   MaximizeBox = false;
   BackColor = Color.FromArgb(17, 23, 31);
   ForeColor = Color.WhiteSmoke;
   Font = new Font("Malgun Gothic", 9F);
   var title = new Label { Text = "로비 슬롯: 열림 · 닫힘 · 컴퓨터", Font = new Font("Malgun Gothic", 17F), AutoSize = false, ForeColor = Color.FromArgb(255, 181, 86) };
   title.SetBounds(24, 18, 600, 36);
   Controls.Add(title);
   var help = new Label { Text = "스타 종료 → 이 창에서 연결 대기 → Battle.net에서 실행 → 방장으로 방 생성\r\n이미 실행 중인 스타에 연결하면 기존 로비 패널에 적용되지 않을 수 있습니다.\r\n현재 단계와 게임 화면을 확인하세요. 설치 원본 파일은 변경하지 않습니다." };
   help.SetBounds(24, 65, 600, 60);
   Controls.Add(help);
   connect.Text = "로비 연결 · 실행 대기";
   connect.SetBounds(24, 138, 600, 40);
   connect.BackColor = Color.FromArgb(150, 94, 35);
   connect.ForeColor = Color.WhiteSmoke;
   connect.FlatStyle = FlatStyle.Flat;
   connect.Click += async delegate { await Connect(); };
   Controls.Add(connect);
   status.Text = "연결 대기";
   status.SetBounds(24, 191, 600, 44);
   status.ForeColor = Color.LightGreen;
   Controls.Add(status);
   history.Multiline = true;
   history.ReadOnly = true;
   history.ScrollBars = ScrollBars.Vertical;
   history.BackColor = Color.FromArgb(11, 14, 20);
   history.ForeColor = Color.Silver;
   history.SetBounds(24, 245, 600, 112);
   Controls.Add(history);
   timer.Interval = 1000;
   timer.Tick += delegate { ReadModuleStatus(); };
   timer.Start();
  }

  internal bool HasActiveMonitor { get { return connecting; } }
  internal Task StartConnection() { return Connect(); }
  internal void ConfigureEmbedded() {
   TopLevel = false;
   FormBorderStyle = FormBorderStyle.None;
   Dock = DockStyle.Fill;
   connect.Visible = false;
   status.SetBounds(24, 138, 600, 44);
   history.SetBounds(24, 198, 600, 124);
   foreach (Control child in Controls) {
    var label = child as Label;
    if (label != null && label.Top == 65) label.Text = "상단 '게임 연결 · 실행 대기'로 로비와 유닛 제어를 함께 연결합니다.\r\n방장으로 생성한 방에서 열림 · 닫힘 · 컴퓨터 슬롯 변경을 사용하세요.\r\n실제 슬롯 상태와 상대 PC의 반영 결과는 게임 화면에서 확인하세요.";
   }
  }
  internal void StopMonitor() {
   if (closing) return;
   closing = true;
   timer.Stop();
   var process = ownedMonitor;
   if (process != null) {
    try { if (!process.HasExited) process.Kill(); } catch (InvalidOperationException) { }
      catch (System.ComponentModel.Win32Exception) { }
   }
  }
  async Task Connect() {
   if (connecting || closing) return;
   connecting = true;
   connect.Enabled = false;
   connect.Text = "연결 상태 확인 중";
   helperId = 0;
   lastLog = null;
   failure = null;
   startupWaiting = false;
   initialStage = 1;
   browsers.Clear();
   history.Clear();
   connectionStartedUtc = DateTime.UtcNow;
   UpdateStatus();
   Process process = null;
   try {
    string connector = Path.Combine(folder, "CefConnector.exe");
    string dll = Path.Combine(folder, "sc_lobby_ui.dll");
    var runningGames = Process.GetProcessesByName("StarCraft");
    bool waitForStartup = runningGames.Length == 0;
    foreach (var game in runningGames) game.Dispose();
    startupWaiting = waitForStartup;
    if (!waitForStartup) history.AppendText("스타가 이미 실행 중입니다. 기존 로비 패널에는 적용되지 않을 수 있습니다." + Environment.NewLine);
    UpdateStatus();
    string mode = waitForStartup ? "--wait" : "--connect";
    var info = new ProcessStartInfo(connector, mode + " --dll \"" + dll + "\"") {
     WorkingDirectory = folder,
     UseShellExecute = false,
     CreateNoWindow = true,
     RedirectStandardOutput = true,
     RedirectStandardError = true,
     StandardOutputEncoding = Encoding.UTF8,
     StandardErrorEncoding = Encoding.UTF8
    };
    process = new Process { StartInfo = info };
    ownedMonitor = process;
    Process thisMonitor = process;
    process.OutputDataReceived += delegate(object sender, DataReceivedEventArgs e) {
     if (e.Data != null) QueueLine(thisMonitor, e.Data, false);
    };
    process.ErrorDataReceived += delegate(object sender, DataReceivedEventArgs e) {
     if (e.Data != null) QueueLine(thisMonitor, e.Data, true);
    };
    if (!process.Start()) throw new InvalidOperationException("상태 확인 프로그램을 실행하지 못했습니다.");
    process.BeginOutputReadLine();
    process.BeginErrorReadLine();
    await Task.Run(delegate { thisMonitor.WaitForExit(); });
    if (closing) return;
    ReadModuleStatus();
    if (process.ExitCode != 0 && failure == null) failure = "연결 실패 · 아래 오류 기록을 확인하세요";
    if (failure != null) UpdateStatus();
    else {
     status.ForeColor = Color.Silver;
     status.Text = "상태 확인 종료 · 다시 연결하려면 스타의 실행 상태를 확인하세요";
    }
   } catch (Exception ex) {
    if (!closing) {
     failure = "연결 실패: " + ex.Message;
     history.AppendText(ex.Message + Environment.NewLine);
     UpdateStatus();
    }
   } finally {
    if (ReferenceEquals(ownedMonitor, process)) ownedMonitor = null;
    if (process != null) process.Dispose();
    if (!closing) {
     connecting = false;
     connect.Enabled = true;
     connect.Text = "로비 연결 · 실행 대기";
    }
   }
  }

  void QueueLine(Process source, string line, bool isError) {
   if (closing || IsDisposed || !IsHandleCreated) return;
   try {
    BeginInvoke((Action)delegate {
     if (closing || !ReferenceEquals(ownedMonitor, source)) return;
     history.AppendText(line + Environment.NewLine);
     if (history.TextLength > 60000) history.Text = history.Text.Substring(history.TextLength - 40000);
     ParseLine(line, isError);
    });
   } catch (InvalidOperationException) { }
  }

  void ParseLine(string line, bool isError) {
   var pidMatch = Regex.Match(line, @"CEF browser PID=(\d+)");
   int parsedId;
   if (pidMatch.Success && int.TryParse(pidMatch.Groups[1].Value, out parsedId) && parsedId > 0) {
    helperId = parsedId;
    startupWaiting = false;
    UpdateStatus();
   }
   if (line.Contains("STARTUP_WAIT:")) {
    startupWaiting = true;
    UpdateStatus();
   }
   if (isError || line.StartsWith("FAILED:", StringComparison.Ordinal)) {
    failure = "연결 오류 · 아래 오류 기록을 확인하세요";
    UpdateStatus();
    return;
   }
   if (line.Contains("External module load completed.") || line.Contains("This module is already loaded;")) {
    initialStage = Math.Max(initialStage, 2);
    UpdateStatus();
   }
   var eventMatch = Regex.Match(line, @"^\d{2}:\d{2}:\d{2} pid=(\d+) ([A-Z_]+) value=(-?\d+)$");
   int eventPid, value;
   if (!eventMatch.Success || !int.TryParse(eventMatch.Groups[1].Value, out eventPid) || eventPid != helperId ||
       !int.TryParse(eventMatch.Groups[3].Value, NumberStyles.Integer, CultureInfo.InvariantCulture, out value)) return;
   string name = eventMatch.Groups[2].Value;
   if (name == "STATE_PHASE" || name == "INITIALIZATION_STATE") {
    if (value >= 100) failure = "모듈 초기화 오류 (단계 " + value + ") · 아래 기록을 확인하세요";
    else if (value == 5) initialStage = Math.Max(initialStage, 3);
    else if (value >= 2 && value <= 4) initialStage = Math.Max(initialStage, 2);
   }
   else if (name == "MODULE_LOADED" || name == "CEF_2357_API_CONFIRMED") initialStage = Math.Max(initialStage, 2);
   else if (name == "IAT_DATA_INTERCEPT_READY") initialStage = Math.Max(initialStage, 3);
   else if (name == "BROWSER_CAPTURED" || name == "LOBBY_BROWSER_CAPTURED") AdvanceBrowser(value, 4);
   else if (name == "BOOTSTRAP_QUEUED") AdvanceBrowser(value, 5);
   else if (name == "JS_READY") AdvanceBrowser(value, 6);
   else if (name == "LOBBY_FRAME_LEFT" || name == "NON_LOBBY_BROWSER_TIMEOUT" || name == "BROWSER_MONITOR_STOPPED") browsers.Remove(value);
   else if (name == "JS_ERROR" || name == "BOOTSTRAP_MISSING_OR_INVALID" || name == "CEF_NOT_LOADED" || name == "TARGET_PROCESS_REJECTED" ||
            name == "UI_TASK_REJECTED" || name == "LOBBY_FRAME_NOT_FOUND" || name == "CREATE_BROWSER_RETURNED_NULL" ||
            name.EndsWith("_FAILED", StringComparison.Ordinal) || name.EndsWith("_UNSUPPORTED", StringComparison.Ordinal) ||
            name.EndsWith("_UNAVAILABLE", StringComparison.Ordinal)) {
    failure = "모듈 오류: " + name + " (" + value + ") · 아래 기록을 확인하세요";
   }
   UpdateStatus();
  }

  void AdvanceBrowser(int id, int stage) {
   int previous;
   if (!browsers.TryGetValue(id, out previous) || stage > previous) browsers[id] = stage;
  }

  void UpdateStatus() {
   if (closing) return;
   if (failure != null) {
    status.ForeColor = Color.FromArgb(255, 181, 86);
    status.Text = failure;
    return;
   }
   int stage = initialStage;
   foreach (int browserStage in browsers.Values) stage = Math.Max(stage, browserStage);
   status.ForeColor = Color.LightGreen;
   if (startupWaiting && stage < 2) {
    status.Text = "스타 실행 대기 · Battle.net에서 스타를 실행하세요";
    return;
   }
   switch (stage) {
    case 1: status.Text = "게임 버전·원본 확인 후 로비 연결 중"; break;
    case 2: status.Text = "외부 모듈 로드 확인 · 메뉴 적용은 아직 확인되지 않았습니다"; break;
    case 3: status.Text = "브라우저 생성 감시 준비 · 로비 패널 포착 대기"; break;
    case 4: status.Text = "브라우저 포착 · 로비 패널 확인 중"; break;
    case 5: status.Text = "로비 스크립트 전달 · 적용 응답 대기"; break;
    case 6: status.Text = "로비 스크립트 응답 확인 · 슬롯 변경 동작은 게임 화면에서 확인하세요"; break;
    default: status.Text = "연결 대기"; break;
   }
  }

  void ReadModuleStatus() {
   if (closing || !connecting || helperId <= 0) return;
   try {
    using (var helper = Process.GetProcessById(helperId)) {
     if (helper.HasExited) { HelperExited(); return; }
    }
    string path = Path.Combine(logFolder, "loader-status-" + helperId + ".txt");
    if (!File.Exists(path) || File.GetLastWriteTimeUtc(path) < connectionStartedUtc) return;
    string content;
    using (var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite))
    using (var reader = new StreamReader(file)) content = reader.ReadToEnd();
    if (content == lastLog) return;
    lastLog = content;
    foreach (string line in content.Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries)) ParseLine(line, false);
   } catch (IOException) { }
     catch (UnauthorizedAccessException) { }
     catch (ArgumentException) { HelperExited(); }
     catch (InvalidOperationException) { HelperExited(); }
  }

  void HelperExited() {
   helperId = 0;
   startupWaiting = false;
   browsers.Clear();
   initialStage = 0;
   failure = null;
   status.ForeColor = Color.Silver;
   status.Text = "게임 브라우저 종료됨 · 연결 대기";
  }

  protected override void OnFormClosing(FormClosingEventArgs e) {
   base.OnFormClosing(e);
   if (!e.Cancel) StopMonitor();
  }
  protected override void OnFormClosed(FormClosedEventArgs e) {
   timer.Dispose();
   var process = ownedMonitor;
   ownedMonitor = null;
   if (process != null) process.Dispose();
   base.OnFormClosed(e);
  }
 }
}