# LiloLauncher

在项目根目录运行：

```powershell
pnpm tauri build --no-bundle
dotnet publish launcher/LiloLauncher.csproj -c Release -o launcher/publish
Copy-Item launcher/publish/LiloLauncher.exe .
```

以后双击项目根目录下的 `LiloLauncher.exe` 即可启动 Lilo。应用已运行时，启动器会激活现有窗口；没有 release 包时，会后台执行完整的 `pnpm app`（不会错误地直接运行依赖 Vite 的 debug 版）。

开发模式启动日志保存在 `%TEMP%\Lilo-startup.log`。修改应用后，重新运行 `pnpm tauri build --no-bundle` 更新启动器使用的正式版。
