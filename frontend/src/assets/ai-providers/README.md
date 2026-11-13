# AI 服务商图标

内置 AI 服务商（见 `core/src/ai/catalog.rs`）的标识，随前端打包，由 `features/settings/ai-provider-icon.tsx` 按文件名（即服务商 id）取用。

- 来源：[models.dev](https://github.com/sst/models.dev) 的服务商图标集（`https://models.dev/logos/<id>.svg`，MIT 许可）。
- 下载日期：2026-11-13。
- 这些是 models.dev 重绘的单色 `currentColor` 图形，只作为 CSS 遮罩（`mask-image`）使用，颜色跟随主题，不当作原始品牌素材；SVG 不会作为标记插入页面。
- 与[邮箱服务商图标规范](../providers/README.md)无关：那套规则只管邮箱服务商，要求官方原件且不得改色。
- `ollama.svg` 取自 models.dev 的 `ollama-cloud`，因为 models.dev 的 `ollama` 只是通用占位图。
- 已逐个确认为静态 SVG：无脚本、无外部引用、无样式表。
- 不在此处的服务商（含自定义）在运行时由后端按需获取，失败时显示名称首字母。

| 文件 | SHA-256 |
| --- | --- |
| `alibaba-cn.svg` | `bf4119a0f34265479d285d646888ed73db4c110fcea66456f9e88cc7085d68db` |
| `anthropic.svg` | `7dbf450ab558c459b9a0f86d265ae6063560cb164189f78ce705d330f8d1426c` |
| `deepseek.svg` | `fdc7613fa261eab83bccf6a2e087cf4d83b9050e9222a874689447ad901a84d5` |
| `lmstudio.svg` | `461acdf34bf96cb90b9767b521aefdd04ca7d40b5ccda88f0a0ffc43aaf736d4` |
| `minimax-cn.svg` | `f3200b68c5d7d6a0b25f6c097e6a818808fe8b5aa17b7190ef702ac8a17988de` |
| `moonshotai-cn.svg` | `fb10fd4231a3b3785f866d3c78ff521a79d614a8cf87c369575cf916078d5141` |
| `ollama.svg` | `b2e03b786987a1a66a5144f0373d33320a256a6e4dad391593f7b16232d40c43` |
| `openai.svg` | `a495ae1dee53ebf3eab642acb0c6e564ea23ceb7c1565c826f480cb2bf32beda` |
| `openrouter.svg` | `5087a8510fa7e339c0fd9f658e76362d00d39a94a75b90d5fb42b8b28c6b964f` |
| `siliconflow-cn.svg` | `52e21b0870e52e402c9e57a3fec66c740bf5b24c1689a9428f5769d2abe74c69` |
| `volcengine.svg` | `c1e06b7f395d83debe15ac8c692a68f1ea8d4cf80a49eecf8e0211ccb2fcb8d1` |
| `zhipuai.svg` | `57d355dafab000caa3386066f9df7e179da577729295cb4600d42cec998bc712` |
