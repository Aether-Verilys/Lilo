using System.Diagnostics;

static class Program
{

    [STAThread]
    static void Main()
    {
        try
        {
            Launch();
        }
        catch (Exception ex)
        {
            MessageBox.Show($"无法启动 Lilo：\n{ex.Message}", "Lilo", MessageBoxButtons.OK, MessageBoxIcon.Error);
        }
    }

    static void Launch()
    {
        var root = FindProjectRoot();
        if (root is null) { MessageBox.Show("请将 LiloLauncher.exe 放在 Lilo 项目根目录。", "Lilo"); return; }

        // 由应用的单实例插件恢复主窗口，不能用 MainWindowHandle 猜测隐藏窗口。
        // Debug 版依赖 localhost:1420 的 Vite 服务，不能由双击启动器直接运行。
        // 只有可独立运行的 release 包才直接启动；否则走完整的 Tauri 开发命令。
        var app = Path.Combine(root, "src-tauri", "target", "release", "lilo.exe");
        if (File.Exists(app))
        {
            Process.Start(new ProcessStartInfo(app) { WorkingDirectory = root, UseShellExecute = true });
            return;
        }

        var pnpm = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "pnpm", "pnpm.CMD");
        if (!File.Exists(pnpm)) pnpm = "pnpm.cmd";
        // cmd 用双引号包围命令和路径，不支持用反斜杠转义引号。
        // 开发模式输出保存在日志中，避免隐藏窗口后启动错误无处可查。
        var log = Path.Combine(Path.GetTempPath(), "Lilo-startup.log");
        using var process = Process.Start(new ProcessStartInfo("cmd.exe", $"/d /s /c \"\"{pnpm}\" app > \"{log}\" 2>&1\"")
        {
            WorkingDirectory = root,
            UseShellExecute = false,
            CreateNoWindow = true,
            WindowStyle = ProcessWindowStyle.Hidden,
        });
        if (process is null) throw new InvalidOperationException("未能创建启动进程。");
        if (process.WaitForExit(2000) && process.ExitCode != 0)
            throw new InvalidOperationException($"开发模式启动失败，请查看日志：{log}");
    }

    static string? FindProjectRoot()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null)
        {
            if (File.Exists(Path.Combine(dir.FullName, "package.json")) && Directory.Exists(Path.Combine(dir.FullName, "src-tauri"))) return dir.FullName;
            dir = dir.Parent;
        }
        return null;
    }
}
