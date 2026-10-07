using System;
using ScMultiTest;

internal static class MenuWaitTests
{
    private const int Pid = 701;
    private const string WaitingMessage = "Waiting for in-game UI callbacks... pending key_down index=0";
    private const string TimeoutMessage = "Game UI callbacks stayed empty... key_down index=0";
    private const string Waiting = "SCMULTI1\t701\t0\t0\t0\t0\tWAITING\t" + WaitingMessage;
    private const string Failed = "SCMULTI1\t701\t0\t0\t0\t0\tFAILED\t" + TimeoutMessage;
    private static int passed;
    private static void Check(bool value, string name)
    {
        if (!value) throw new Exception("FAILED: menu wait: " + name);
        passed++;
    }
    internal static int Run()
    {
        passed = 0;
        Snapshot waiting = Snapshot.Parse(Waiting, Pid);
        Check(waiting.State == "WAITING" && !waiting.Active && waiting.Count == 0
            && waiting.Message == WaitingMessage, "menu waiting remains inactive and retains native detail");
        Check(StatusText.Explain(WaitingMessage)
            == "메뉴·로비 연결 대기 · 테스트 게임에 들어가면 연결을 이어갑니다.",
            "waiting prefix translated with appended callback detail");
        DateTime now = new DateTime(2026, 10, 4, 3, 54, 39, DateTimeKind.Utc);
        DateTime start = now.AddMinutes(-2);
        Check(Snapshot.IsFresh(now.AddMilliseconds(-2999), start, now),
            "fresh waiting heartbeat is accepted without requiring READY");
        Check(!Snapshot.IsFresh(now.AddSeconds(-3), start, now),
            "waiting heartbeat still expires at freshness boundary");
        Check(TestMonitor.DecideExistingConnection(null, waiting.State)
            == TestMonitor.ExistingConnectionAction.WaitForModule,
            "waiting retry cannot resume a ready connection");
        Snapshot failed = Snapshot.Parse(Failed, Pid);
        Check(failed.State == "FAILED" && !failed.Active && failed.Message == TimeoutMessage
            && StatusText.Explain(failed.Message)
                == "게임 안에서도 연결 위치가 준비되지 않았습니다. 초기화 오류 기록을 확인하세요."
            && TestMonitor.DecideExistingConnection(null, failed.State)
                == TestMonitor.ExistingConnectionAction.RestartGame,
            "timeout retains failed state and native detail while translating prefix");
        Check(StatusText.Explain("Select exactly one owned unit first") == "같은 종류의 본인 유닛 1개 또는 여러 개를 선택한 뒤 백틱(`)을 누르세요."
            && TestMonitor.DecideExistingConnection(null, "READY")
                == TestMonitor.ExistingConnectionAction.ResumeReady,
            "ready behavior stays intact and selection guidance accepts one or more same-type units");
        try
        {
            Snapshot.Parse("SCMULTI1\t701\t1\t15\t37\t0\tWAITING\t" + WaitingMessage, Pid);
            throw new Exception("FAILED: menu wait accepted active pending initialization");
        }
        catch (FormatException) { passed++; }
        return passed;
    }
}
