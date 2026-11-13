// The Windows door: a named pipe, opened as a file with CreateFileW, the same
// way sdk/rust/elo_overlay.rs opens it. P/Invoke rather than
// System.IO.Pipes, so it behaves the same under Mono and IL2CPP.
//
// ⚠ The timeout bounds the CONNECT, not the reads (sdk/PROTOCOL.md). A
// launcher that accepts and never answers hangs the calling thread, which is
// one more reason every call goes through Elo's worker thread.

using System;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace EloPros
{
    static class WindowsPipe
    {
        const uint GENERIC_READ = 0x80000000;
        const uint GENERIC_WRITE = 0x40000000;
        const uint OPEN_EXISTING = 3;
        // The launcher may identify us; it may not act as us. Without this a
        // pipe server holding SeImpersonatePrivilege could.
        const uint SECURITY_SQOS_PRESENT = 0x00100000;
        const uint SECURITY_IDENTIFICATION = 1 << 16;
        const uint PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;
        const uint TOKEN_QUERY = 0x0008;
        const int TokenUser = 1;

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern bool WaitNamedPipeW(string name, uint timeoutMs);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern SafeFileHandle CreateFileW(string name, uint access, uint share, IntPtr security,
            uint disposition, uint flags, IntPtr template);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool GetNamedPipeServerProcessId(SafeFileHandle pipe, out uint pid);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);

        [DllImport("kernel32.dll")]
        static extern IntPtr GetCurrentProcess();

        [DllImport("kernel32.dll")]
        static extern bool CloseHandle(IntPtr h);

        [DllImport("advapi32.dll", SetLastError = true)]
        static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);

        [DllImport("advapi32.dll", SetLastError = true)]
        static extern bool GetTokenInformation(IntPtr token, int cls, IntPtr info, int len, out int returned);

        [DllImport("advapi32.dll")]
        static extern bool EqualSid(IntPtr a, IntPtr b);

        public static Stream Open(string path, TimeSpan timeout)
        {
            uint ms = (uint)Math.Min(uint.MaxValue, Math.Max(0, timeout.TotalMilliseconds));
            // Not fatal on its own: the pipe may already have a free instance.
            WaitNamedPipeW(path, ms);
            var h = CreateFileW(path, GENERIC_READ | GENERIC_WRITE, 0, IntPtr.Zero, OPEN_EXISTING,
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, IntPtr.Zero);
            if (h.IsInvalid)
            {
                int err = Marshal.GetLastWin32Error();
                h.Dispose();
                throw new IOException($"cannot open {path} (error {err})");
            }
            // The pipe name is predictable and answers for whoever created it
            // first, so check the process serving it runs as this user.
            if (!ServerIsUs(h))
            {
                h.Dispose();
                throw new IOException($"the pipe at {path} is held by another user, not your launcher");
            }
            return new FileStream(h, FileAccess.ReadWrite, 1, false);
        }

        static bool ServerIsUs(SafeFileHandle pipe)
        {
            if (!GetNamedPipeServerProcessId(pipe, out uint pid) || pid == 0) return false;
            IntPtr server = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
            if (server == IntPtr.Zero) return false;
            IntPtr theirs = IntPtr.Zero, ours = IntPtr.Zero;
            try
            {
                theirs = UserOf(server);
                ours = UserOf(GetCurrentProcess());
                // TOKEN_USER starts with SID_AND_ATTRIBUTES, whose first field is the PSID.
                return theirs != IntPtr.Zero && ours != IntPtr.Zero
                    && EqualSid(Marshal.ReadIntPtr(theirs), Marshal.ReadIntPtr(ours));
            }
            finally
            {
                if (theirs != IntPtr.Zero) Marshal.FreeHGlobal(theirs);
                if (ours != IntPtr.Zero) Marshal.FreeHGlobal(ours);
                CloseHandle(server);
            }
        }

        /// <summary>The TOKEN_USER of a process, in a buffer the caller frees.</summary>
        static IntPtr UserOf(IntPtr process)
        {
            if (!OpenProcessToken(process, TOKEN_QUERY, out IntPtr token)) return IntPtr.Zero;
            try
            {
                GetTokenInformation(token, TokenUser, IntPtr.Zero, 0, out int len);
                if (len <= 0) return IntPtr.Zero;
                IntPtr buf = Marshal.AllocHGlobal(len);
                if (!GetTokenInformation(token, TokenUser, buf, len, out _))
                {
                    Marshal.FreeHGlobal(buf);
                    return IntPtr.Zero;
                }
                return buf;
            }
            finally
            {
                CloseHandle(token);
            }
        }
    }
}
