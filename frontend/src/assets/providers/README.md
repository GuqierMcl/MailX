# 邮箱服务商图标

这里保存随前端打包的第三方服务商标识，用于识别用户选择的邮箱服务商；不属于 MailX 自有品牌，也不由 `branding/logo.svg` 派生。应用运行时不从服务商站点加载这些图标。

2026-10-30 起采用官方发布的原始素材：栅格图以本地 PNG 保存，官方提供静态 SVG 的（Gmail）直接保存原文件。保留完整图形、字形、配色、背景与长宽比例，不再提取部分数字、删去标识文字或使用 VTracer 描摹为 SVG。CSS 只做等比显示，不对图像重新着色。

## 选材和处理规范

本节是新增或替换服务商标识的维护规范，适用于应用界面和仓库文档中的服务商图标。MailX 自有品牌沿用 [branding/README.md](../../../../branding/README.md) 的源稿与生成流程。

### 官方来源与产品对应

- 优先选择服务商正式发布、适合目标尺寸的产品图标；可使用官方品牌素材页、产品官网实际引用的图片、favicon、manifest 或 CSS 内嵌图片。原生 PNG、JPEG、ICO 和静态 SVG 均可采用，不为统一格式强制转换成 SVG。
- 官网没有合适素材时，可使用由官网链接、且发布者身份与服务商一致的官方应用商店图标。记录完整来源链，不能只凭图片外观或文件名确认来源。
- 图标必须对应所标识的邮箱产品。集团标识、同公司其他软件和邮箱地址所属服务不自动互相替代；例如 QQ 即时通信、QQ 邮箱和 Foxmail 桌面客户端应分别识别。
- 不使用无官方来源证据的图标聚合站、个人重绘稿或第三方仓库副本。没有合适的官方素材时，先使用通用邮件图标配服务商名称；小尺寸不清晰时继续寻找官方紧凑版本，不自行简化原图。

### 保留原图

| 操作 | 规则 |
| --- | --- |
| 保存或提取 | 优先保留官方文件字节。CSS 内嵌图只解码 Base64；精灵图只提取官网样式明确指定的完整图标区域，不能自行截取其中的数字、文字或图案。 |
| 格式转换 | 仅在工程需要时做忠实的栅格解码或无损编码，例如 JPEG / ICO 转 PNG；保留原尺寸、全部像素、透明度与已有色彩配置，核对解码后内容一致，记录原文件和结果的哈希。格式转换不能用于重设计。 |
| 界面适配 | 使用 `object-contain` 等比显示整张图片，留白和账号颜色放在容器上。保留图片自带背景；深浅主题可选择官方提供的对应版本并分别记录来源。 |
| 禁止改绘 | 不使用 VTracer、手工路径、系统字体或生成式图像重画标识；不删文字、截掉品牌组成部分、去背景、改色、反色、变形、补画或重新拼装。也不通过 CSS 滤镜、遮罩、`object-cover` 或裁切容器达到同样效果。 |
| SVG | 只使用官方原生静态 SVG，不把 PNG 嵌入 SVG 充当矢量原件。检查没有脚本、外部资源或运行时字体依赖；原文件不适合静态打包时，选用其他官方静态格式或通用图标。 |

### 落盘与验证

1. 在本目录保存素材并更新本说明：记录对应产品、官方页面、直接资源 URL 或 CSS 选择器、核验日期、原始格式、像素尺寸、本地 SHA-256，以及所有必要提取或格式转换步骤。
2. 使用本地导入随应用打包，不在运行时热链官网图片。替换后更新服务商登记和中英文 README 等引用，移除已不再使用的旧素材；不要将服务商图标交给 MailX 自有品牌的 `icons:generate` 生成。
3. 检查实际使用的小尺寸（当前包含 14 / 18 / 20 / 24 / 32 / 40 px）以及浅深背景，确认图像完整、比例正确、没有用背景处理掩盖裁切；细小文字可能不可读，旁边的服务商名称仍须清楚。
4. 验证来源文件字节或解码像素、哈希、文档链接和生产资源打包；运行受影响的服务商 / 头像现有测试、前端 lint 与构建。仅图片和文档变更无需重跑真实邮箱收发测试。验证结果与限制写入 [验证记录](../../../../docs/development/validation.md)。

## QQ 邮箱

- 文件：`qqmail.png`，96 × 96，直接保存官方 PNG 原文件。
- 来源页面：[QQ 邮箱登录页](https://mail.qq.com/)。
- 原图：[QQ 邮箱图标](https://res.wx.qq.com/t/webmail/webmail/res/images/qqmail_favicon_96h.8d124a7.png)。
- 使用 QQ 邮箱专属图标，替换此前来自 QQ 即时通信官网的经典企鹅。QQ 邮箱同时提供 `qq.com` 和 `foxmail.com` 地址，因此共用此图；Foxmail 桌面客户端标识不用于表示邮箱地址。

## 网易 163 / 126 / yeah.net 邮箱

采用[网易邮箱总入口](https://email.163.com/?from=dashi)中服务商切换器实际使用的三个独立圆形图标，均为 100 × 100 PNG。选择官方的彩色圆底、白色字标版本，保留透明四角；在浅深背景和小尺寸下均能区分三个品牌。

三张图来自该页面引用的[官方 CSS](https://mimg.127.net/p/freemail/index/email/css/style.57f97fa3f54499e44d06.css)。仅解码其中的 Base64 数据，文件字节保持原样，没有从完整页眉裁出数字。

| 本地文件 | 官方样式选择器 | 原始素材 |
| --- | --- | --- |
| `netease-163.png` | `.m-login-content .u-urs-login.mail-163 .selector-163` | 100 × 100，红底白色 `163` |
| `netease-126.png` | `.m-login-content .u-urs-login.mail-126 .selector-126` | 100 × 100，绿底白色 `126` |
| `netease-yeah.png` | `.m-login-content .u-urs-login.mail-yeah .selector-yeah` | 100 × 100，红底白色 `yeah` |

## 中国移动 139 邮箱

- 文件：`mail-139.png`，66 × 66 RGBA PNG，保留完整官方图标的蓝色圆底、白色信封和 `139` 标识，不含推广信息。
- 来源链：[139 官方下载页](https://logininterface.mail.10086.cn/download/index.html)的 Android 下载入口指向[官方 APK](https://appdownload.mail.10086.cn/event/static/sys/20200723/PE-V11.6.0-9048001.apk)；APK 内的 `assets/blue/drawable/ic_mail_139.png` 与 `assets/black/drawable/ic_mail_139.png` 字节一致。
- 直接保留 APK 内 PNG 原文件，没有裁切、缩放、补画、去背景或改色；核验日期：2026-11-11。
- PNG 原文件 SHA-256：`837435be15ce7c6d53d6fb52f830bbbae9e3b42a9fe51e2a7cdd2a138cf3f2b1`。

## iCloud Mail

- 文件：`icloud-mail.png`，240 × 240 PNG，直接保存 Apple 官方浅色版原文件；统一用于浅深主题，保留蓝色圆角背景、白色信封及信封上的细小文字，没有裁切、改色或格式转换。
- 来源页面：[iCloud Mail](https://www.icloud.com/mail/)，核验日期：2026-11-06。
- 来源链：页面声明 `<base href="/system/icloud.com/2640Build24/en-us/">`，并引用[官方 `main.js`](https://www.icloud.com/system/icloud.com/2640Build24/en-us/main.js)。其中 AppIcon 的 `case "mail": case "mail2"` 明确选择 Mail 图标，并在浅色图片的 `srcSet` 中以 `3x` 引用 `../2b0f92bb67c43113d3f94d0d1377573c.png`；相对页面 base 解析得到下方原图 URL。
- 原图：[iCloud Mail 3x PNG](https://www.icloud.com/system/icloud.com/2640Build24/2b0f92bb67c43113d3f94d0d1377573c.png)。本地文件与下载原件字节一致，保留完整图形与原始长宽比例；SHA-256 见下表。

## Gmail

- 文件：`gmail.svg`，192 × 192 静态 SVG，直接保存 Google 官方原文件，1,298 字节；统一用于浅深主题，透明背景，没有修改路径、渐变或尺寸。
- 来源页面：[Gmail 产品页](https://workspace.google.com/products/gmail/)，核验日期：2026-11-08。页面导航中 `alt="Gmail"` 的产品图标引用下方原图。
- 原图：[Gmail 192px SVG](https://www.gstatic.com/images/branding/productlogos/gmail_2026/v2/web/192px.svg)，`content-type: image/svg+xml`，`last-modified: Mon, 09 Mar 2026 22:48:00 GMT`。本地文件与下载原件字节一致；只含路径与内部渐变，没有脚本、外部资源或字体依赖。

## Microsoft Outlook

- 文件：`outlook.svg`，48 × 48 静态 SVG，8,832 字节，直接保存 Microsoft 官方原文件；统一用于浅深主题，透明背景，没有修改路径、渐变或尺寸。用于“Outlook”入口（`microsoft-personal`，Outlook.com、Hotmail、Live 个人邮箱）。
- 来源页面：Microsoft 365 产品图标，由 Microsoft 的 Fluent UI 文档说明：[`M365ProductIconsImplementation.md`](https://github.com/microsoft/fluentui/blob/master/apps/public-docsite/src/pages/Styles/M365ProductIconsPage/docs/web/M365ProductIconsImplementation.md)（`github.com/microsoft/fluentui`，路径 `apps/public-docsite/src/pages/Styles/M365ProductIconsPage/docs/web/`），其中把网页图标指向 `https://res-1.cdn.office.net/files/fabric-cdn-prod_20251008.001/assets/brand-icons/product/svg/{product}_48x1.svg`。核验日期：2026-11-10。
- 原图：[`outlook_48x1.svg`](https://res-1.cdn.office.net/files/fabric-cdn-prod_20251008.001/assets/brand-icons/product/svg/outlook_48x1.svg)，`content-type: image/svg+xml`，`last-modified: Thu, 09 Oct 2025 07:37:12 GMT`。本地文件与下载原件字节一致；只含路径与内部渐变，没有脚本、外部资源或字体依赖。
- 使用条件：依 [Microsoft Fabric Assets License](https://aka.ms/fluentui-assets-license)（`microsoft_fabric_assets_license_agreement_nov_2019.pdf`）第 1(b) 条，可以用这些素材“说明应用与一项或多项 Microsoft 产品和服务集成”；附件 1 允许在不可点击的并列清单中使用图标，旁边须有完整产品名称，图标保持原样、全彩正色版本。MailX 在服务商列表和账号标识里，图标旁边都显示“Outlook”名称，不作改色、反色、裁切或变形，也不让它充当可点击的 Microsoft 标识。许可证条款以原文为准，发布前如需更严格的判断应由权利人确认。

## Microsoft 365 / Exchange（Microsoft 标识）

- 文件：`microsoft.svg`，21 × 21 静态 SVG，343 字节，直接保存 Microsoft 官方原文件（四色方块标识，`<title>MS-SymbolLockup</title>`）；统一用于浅深主题，透明背景，没有修改。用于“Microsoft 365 / Exchange”入口（`microsoft-work`，工作或学校邮箱）。
- 来源页面：Microsoft Learn [Sign in with Microsoft branding guidelines](https://learn.microsoft.com/en-us/entra/identity-platform/howto-add-branding-in-apps)，提供该文件供应用使用，并说明：用户尚未登录时，应称“工作或学校账号”，并用 Microsoft 标识表示这些账号由 Microsoft 管理；不得改动标识。核验日期：2026-11-10。
- 原图：[`ms-symbollockup_mssymbol_19.svg`](https://learn.microsoft.com/en-us/entra/identity-platform/media/howto-add-branding-in-apps/ms-symbollockup_mssymbol_19.svg)。本地文件与下载原件字节一致，只含四个纯色矩形，没有脚本、外部资源或字体依赖。
- 为什么这里用公司标识而不是 Exchange 产品图标：本规范通常不让集团标识替代产品图标，但此入口的情况不同。Microsoft 的 Fluent 产品图标集（可用于应用集成的授权来源）没有 Exchange 图标；Microsoft 365 架构图标包只允许用于图表、培训和文档，不能用于应用界面；Fluent 的 `office` 图标是已停用的旧 Office 标识。而 Microsoft 自己的登录品牌指南要求工作或学校账号使用 Microsoft 标识。因此采用该标识，名称仍写“Microsoft 365 / Exchange”，不单独表示 Exchange 产品。若 Microsoft 之后发布适用于应用的 Exchange 或 Microsoft 365 产品图标，应改用并更新本节。

## 文件核验与维护

QQ 邮箱与网易邮箱的核验日期：2026-10-30；中国移动 139 邮箱的核验日期：2026-11-11；iCloud Mail 的核验日期：2026-11-06；Gmail 的核验日期：2026-11-08；Microsoft Outlook 与 Microsoft 标识的核验日期：2026-11-10。下表记录本地文件的 SHA-256，用于发现素材更新或意外修改。

| 文件 | SHA-256 |
| --- | --- |
| `qqmail.png` | `ea5718ce82f326c827ac6de52bc2e759864650956062faa61fcaf61694d0ab6e` |
| `netease-163.png` | `3c47492e5f0c3ab280571be237468d245111cb24df5454b250138e5ae8134ada` |
| `netease-126.png` | `7a938c619dbeca51d4ece451d4f8b02107435638be5ab6e98fa3ed8c3d694b2d` |
| `netease-yeah.png` | `77f34d1b7c3ac5477cf9a1e79e8c601a990332e9186aeab428e0dd3a82d4fc86` |
| `mail-139.png` | `837435be15ce7c6d53d6fb52f830bbbae9e3b42a9fe51e2a7cdd2a138cf3f2b1` |
| `icloud-mail.png` | `eb04a20229ea2fe673b7b78bf84213176a8ae765bf5c5ec88ac5c2458585f869` |
| `gmail.svg` | `0a242131424b796a26247a8edd41d66fa294da6d177897526a19b7e81aedfdb8` |
| `outlook.svg` | `81bfb24bb128298a9ed0ea6fe8d9ba8cff703b26d6a7cbd18cc4314cecba8f1f` |
| `microsoft.svg` | `929f48f88c8ca7f3f5d294be47ec4caf51acc28ac25340c19a903125d7ecd84a` |

服务商名称及图形属于各自权利人，与 MailX 自有品牌资源分开维护。
