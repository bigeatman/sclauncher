using System;
using System.Threading;
using System.Windows.Forms;

namespace ScMultiTest
{
    internal static class Program
    {
        [STAThread]
        private static void Main()
        {
            bool created;
            using (var mutex = new Mutex(true, @"Local\SCMultiTest.UI.v1", out created))
            {
                if (!created)
                {
                    MessageBox.Show("이전 유닛 제어 도구 또는 SC Launcher가 이미 실행 중입니다. 작업 표시줄 또는 알림 영역(트레이)에서 열어 주세요.", "SC Launcher");
                    return;
                }
                try
                {
                    Application.EnableVisualStyles();
                    Application.SetCompatibleTextRenderingDefault(false);
                    Application.Run(new MainForm());
                }
                finally { mutex.ReleaseMutex(); }
            }
        }
    }
}
