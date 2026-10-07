using System;
using ScMultiTest;

internal static class ConnectionRetryTests
{
    internal static int Run()
    {
        int passed = 0;
        Action<string, string, TestMonitor.ExistingConnectionAction> check = delegate(string attachFailure, string nativeState, TestMonitor.ExistingConnectionAction expected)
        {
            if (TestMonitor.DecideExistingConnection(attachFailure, nativeState) != expected)
                throw new Exception("FAILED: existing connection retry: " + (nativeState ?? "no response"));
            passed++;
        };
        check(null, "FAILED", TestMonitor.ExistingConnectionAction.RestartGame);
        check("Load failed", "READY", TestMonitor.ExistingConnectionAction.RestartGame);
        check("Load failed", null, TestMonitor.ExistingConnectionAction.RestartGame);
        check(null, "READY", TestMonitor.ExistingConnectionAction.ResumeReady);
        check(null, "WAITING", TestMonitor.ExistingConnectionAction.WaitForModule);
        check(null, null, TestMonitor.ExistingConnectionAction.WaitForModule);
        return passed;
    }
}