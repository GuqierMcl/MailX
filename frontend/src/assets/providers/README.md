# 邮箱服务商图标

这里保存随前端打包的第三方服务商标识，仅用于识别用户选择的邮箱服务商；不属于 MailX 自有品牌，也不由 `branding/logo.svg` 派生。应用运行时不从服务商站点加载这些图标。

四个服务商标识统一使用 SVG 路径，文件不包含内嵌位图、脚本或外部资源；已用 SVGO 压缩，并检查 24 / 32 / 40 px 的显示。官方提供的源素材为 PNG，本地版本从其品牌主体提取轮廓，使用 VTracer 转换成路径并保留主体配色；因此是衍生的矢量版本，并非官方原生 SVG。

## QQ 邮箱

- 文件：`qqmail.svg`，使用 QQ 经典黑白企鹅、红围巾与黄色嘴脚，替换此前的 QQ 邮箱圆形 Q 标识。
- 来源页面：[QQ 官方首页](https://im.qq.com/index/)。
- 原图：[页面声明的 QQ 企鹅图标](https://static-res.qq.com/static-res/imqq/qq-logo.png)。
- 核验日期：2026-10-28；已检查首页 HTML 的图标引用、下载原图并比对本地路径渲染。
- QQ 企鹅在这里用于识别 QQ 邮箱 / Foxmail 预设，不代表 MailX 是 QQ 客户端。

## 网易 163 / 126 / yeah.net 邮箱

三个账号入口采用各自的品牌主体：红色 `163`、绿色 `126` 和红色 `yeah`，不再共用“易”字。省略官网页眉中的“网易免费邮”、网址及 yeah.net 的灰色尾缀，以保证小尺寸头像可识别；不重画数字或使用系统字体替代原有字形。

| 本地文件 | 官方来源页面 | 源素材 |
| --- | --- | --- |
| `netease-163.svg` | [163 登录页](https://mail.163.com/) | [官方样式](https://mimg.127.net/p/freemail/index/unified/static/2026/css/style.52d41eed807f372e4154.css) 的 `.header-163logo` 内嵌 PNG，283 × 60，取左侧 120 × 60 的数字主体 |
| `netease-126.svg` | [126 登录页](https://mail.126.com/) | [官方样式](https://mimg.127.net/p/freemail/index/unified/static/2026/css/style.f725ce8b2ece3e1a366f.css) 的 `.header-126logo` 内嵌 PNG，292 × 62，取左侧 121 × 62 的数字主体 |
| `netease-yeah.svg` | [yeah.net 登录页](https://mail.yeah.net/) | [官方样式](https://mimg.127.net/p/freemail/index/unified/static/2026/css/style.374db7155d792da4f36f.css) 的 `.header-yeahlogo` 内嵌 PNG，251 × 68，提取左侧红色 yeah 主体 |

核验日期：2026-10-28。已核对三个登录页各自使用的 CSS、原图与本地矢量显示。三个 SVG 使用贴近主体的 viewBox，保留透明留白与原有长宽比例；服务商列表和账号头像沿用同一资源与每账号 `useProviderIcon` 偏好。

## 权益与维护

QQ 邮箱、QQ 企鹅、网易邮箱名称、图形及相关标识属于对应服务商及其权利人。此处仅记录来源和处理方式，不授予商标或图形许可，不适用 MailX 自有品牌资源的授权说明。

替换资源时核对官方来源与图像内容，保留清晰的小尺寸识别，并同步更新本说明。不要在 SVG 中嵌入 PNG 来代替矢量转换，也不要加入脚本、外部样式或远程链接。
