using System;
using System.ComponentModel;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Security.Cryptography;

namespace ScMultiTest
{
    internal static class Native
    {
        [StructLayout(LayoutKind.Sequential)] internal struct Rect { public int Left, Top, Right, Bottom; }
        [StructLayout(LayoutKind.Sequential)] internal struct Point { public int X, Y; }
        [DllImport("user32.dll")] internal static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] internal static extern bool IsIconic(IntPtr window);
        [DllImport("user32.dll")] internal static extern bool GetClientRect(IntPtr window, out Rect rect);
        [DllImport("user32.dll")] internal static extern bool ClientToScreen(IntPtr window, ref Point point);
        [DllImport("user32.dll")] internal static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int cx, int cy, uint flags);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern bool IsWow64Process(IntPtr process, out bool wow64);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern bool GetProcessTimes(IntPtr process, out long creation, out long exit, out long kernel, out long user);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr VirtualAllocEx(IntPtr process, IntPtr address, UIntPtr bytes, uint allocation, uint protection);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern bool VirtualFreeEx(IntPtr process, IntPtr address, UIntPtr bytes, uint operation);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern bool WriteProcessMemory(IntPtr process, IntPtr address, byte[] buffer, UIntPtr size, out UIntPtr written);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)] private static extern IntPtr GetModuleHandleW(string name);
        [DllImport("kernel32.dll", CharSet = CharSet.Ansi, ExactSpelling = true)] private static extern IntPtr GetProcAddress(IntPtr module, string name);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr CreateRemoteThread(IntPtr process, IntPtr attributes, UIntPtr stack, IntPtr start, IntPtr parameter, uint flags, IntPtr threadId);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern uint WaitForSingleObject(IntPtr handle, uint timeout);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern bool GetExitCodeThread(IntPtr thread, out uint code);
        [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);

        internal static bool ClientBounds(IntPtr window, out Rectangle bounds)
        {
            bounds = Rectangle.Empty;
            Rect rect; Point origin = new Point();
            if (window == IntPtr.Zero || IsIconic(window) || !GetClientRect(window, out rect) || !ClientToScreen(window, ref origin)) return false;
            if (rect.Right < 320 || rect.Bottom < 200) return false;
            bounds = new Rectangle(origin.X, origin.Y, rect.Right, rect.Bottom);
            return true;
        }

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr OpenEvent(uint desiredAccess, bool inherit, string name);
        [DllImport("kernel32.dll", SetLastError = true)] private static extern bool SetEvent(IntPtr handle);

        internal static void Heartbeat(int pid)
        {
            if (pid <= 0) return;
            IntPtr handle = OpenEvent(0x0002, false, @"Local\SCMultiTest.Heartbeat." + pid);
            if (handle == IntPtr.Zero) return; // The module may still be loading.
            try { SetEvent(handle); }
            finally { CloseHandle(handle); }
        }
        internal static void StopSelection(int pid)
        {
            if (pid <= 0) throw new InvalidOperationException("연결된 게임이 없습니다.");
            IntPtr handle = OpenEvent(0x0002, false, @"Local\SCMultiTest.Stop." + pid);
            if (handle == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "해제 신호를 보낼 수 없습니다. 게임에서 백틱 키로 해제하세요.");
            try { if (!SetEvent(handle)) throw new Win32Exception(Marshal.GetLastWin32Error()); }
            finally { CloseHandle(handle); }
        }

        private static string HashFile(string path)
        {
            using (var stream = File.OpenRead(path))
            using (var sha = SHA256.Create()) return BitConverter.ToString(sha.ComputeHash(stream)).Replace("-", "");
        }
        internal static bool CheckExistingModule(Process target, string dll)
        {
            if (!File.Exists(dll) || !File.Exists(dll + ".sha256")) throw new FileNotFoundException("유닛 제어 모듈 또는 해시 파일이 없습니다.", dll);
            string expected = File.ReadAllText(dll + ".sha256").Trim();
            if (!String.Equals(HashFile(dll), expected, StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("유닛 제어 모듈의 해시가 일치하지 않습니다.");
            foreach (ProcessModule module in target.Modules)
            {
                if (!String.Equals(module.ModuleName, "sc_multi_test.dll", StringComparison.OrdinalIgnoreCase)) continue;
                if (!String.Equals(HashFile(module.FileName), expected, StringComparison.OrdinalIgnoreCase))
                    throw new InvalidOperationException("이전 유닛 제어 모듈이 연결돼 있습니다. 스타를 완전히 종료한 뒤 새 런처로 다시 연결하세요.");
                return true;
            }
            return false;
        }

        // Run off the UI thread after explicit user connection.
        internal static void Attach(Process target, string dll)
        {
            if (!File.Exists(dll)) throw new FileNotFoundException("테스트 모듈 파일이 없습니다.", dll);
            if (CheckExistingModule(target, dll)) return;
            string gameRoot = OriginalFiles.ValidateExternalPaths(target.MainModule.FileName, Path.GetDirectoryName(Path.GetFullPath(dll)), TestMonitor.DataDirectory);
            dll = OriginalFiles.ValidateExternalFile(gameRoot, dll);
            long expectedCreation = target.StartTime.ToUniversalTime().ToFileTimeUtc();
            if (target.HasExited) throw new InvalidOperationException("게임이 종료되었습니다.");
            IntPtr process = OpenProcess(0x0002 | 0x0400 | 0x0008 | 0x0010 | 0x0020, false, target.Id);
            if (process == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "게임 연결 권한을 확인하세요.");
            IntPtr remote = IntPtr.Zero, thread = IntPtr.Zero;
            bool finished = false;
            try
            {
                long creation, exit, kernel, user;
                if (!GetProcessTimes(process, out creation, out exit, out kernel, out user)) throw new Win32Exception(Marshal.GetLastWin32Error(), "게임 프로세스 식별 확인에 실패했습니다.");
                if (creation != expectedCreation) throw new InvalidOperationException("게임 프로세스가 종료되거나 변경되었습니다. 다시 연결하세요.");
                bool wow64;
                if (!IsWow64Process(process, out wow64) || wow64) throw new InvalidOperationException("64비트 스타크래프트만 지원합니다.");
                IntPtr localLoader = GetProcAddress(GetModuleHandleW("kernel32.dll"), "LoadLibraryW");
                if (localLoader == IntPtr.Zero) throw new InvalidOperationException("Windows 모듈 로더를 찾지 못했습니다.");
                string owner = null; long offset = 0;
                using (Process self = Process.GetCurrentProcess())
                {
                    foreach (ProcessModule module in self.Modules)
                    {
                        long start = module.BaseAddress.ToInt64();
                        if (localLoader.ToInt64() >= start && localLoader.ToInt64() < start + module.ModuleMemorySize)
                        { owner = module.ModuleName; offset = localLoader.ToInt64() - start; break; }
                    }
                }
                IntPtr loader = IntPtr.Zero;
                foreach (ProcessModule module in target.Modules)
                    if (string.Equals(module.ModuleName, owner, StringComparison.OrdinalIgnoreCase))
                    { loader = new IntPtr(module.BaseAddress.ToInt64() + offset); break; }
                if (loader == IntPtr.Zero) throw new InvalidOperationException("게임의 Windows 로더를 찾지 못했습니다.");
                byte[] bytes = Encoding.Unicode.GetBytes(Path.GetFullPath(dll) + "\0");
                remote = VirtualAllocEx(process, IntPtr.Zero, new UIntPtr((uint)bytes.Length), 0x1000 | 0x2000, 0x04);
                if (remote == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
                UIntPtr written;
                if (!WriteProcessMemory(process, remote, bytes, new UIntPtr((uint)bytes.Length), out written) || written.ToUInt64() != (ulong)bytes.Length)
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                thread = CreateRemoteThread(process, IntPtr.Zero, UIntPtr.Zero, loader, remote, 0, IntPtr.Zero);
                if (thread == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
                finished = WaitForSingleObject(thread, 8000) == 0;
                if (!finished) throw new InvalidOperationException("모듈 로드 시간 초과. 게임을 재시작한 뒤 다시 연결하세요.");
                uint code;
                if (!GetExitCodeThread(thread, out code) || code == 0) throw new InvalidOperationException("테스트 모듈을 로드하지 못했습니다.");
            }
            finally
            {
                if (thread != IntPtr.Zero) CloseHandle(thread);
                if (remote != IntPtr.Zero && (thread == IntPtr.Zero || finished)) VirtualFreeEx(process, remote, UIntPtr.Zero, 0x8000);
                CloseHandle(process);
            }
        }
    }
}
