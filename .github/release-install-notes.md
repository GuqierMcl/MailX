## 安装说明

当前安装包**尚未做代码签名**，系统可能拦截首次启动：

- **macOS**：下载 `.dmg` 并拖入“应用程序”后，右键应用选择“打开”；若提示“已损坏”，在终端执行 `xattr -dr com.apple.quarantine /Applications/MailX.app`。
- **Windows**：SmartScreen 提示时点击“更多信息” → “仍要运行”。
- **Linux**：`.AppImage` 先执行 `chmod +x`；也可使用 `.deb` / `.rpm`。

请用同目录的 `SHA256SUMS.txt` 核对下载文件。
