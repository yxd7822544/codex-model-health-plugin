# 账号模型体检 · Codex Proxy 插件

为 [Codex Proxy RS](https://github.com/zyycn/codex-proxy-rs) 提供独立的账号模型体检页面：选择已有账号与计量 Key，通过原生代理和请求链路运行 **Meow / ModelTrace** 两种行为指纹检测，无需再次填写 API 地址或密钥，也不修改主项目。

[下载本分支最新构建](https://github.com/AlisaCat-S/codex-model-health-plugin/releases/latest) · [反馈问题](https://github.com/R-lmmortal/codex-model-health-plugin/issues) · [插件使用规范](https://github.com/zyycn/codex-proxy-rs/blob/main/docs/plugins.md)

![插件页面（模拟数据，不代表真实检测结果）](docs/screenshots/overview.png)

## 来源与致谢

本插件集成以下项目的检测算法与基线，感谢原作者；不是自行发明的“降智评分器”，也不是上游官方插件。

| 项目 | 集成内容与固定版本 | 许可证 |
| --- | --- | --- |
| [Meow LLM Detector — chen-006](https://github.com/chen-006/meow-llm-detector) | 题目、响应解析、答案归一化与行为概率评分；引擎 4.5.4 / 基线 4.5.3-predictive.2；[提交 dc608d5](https://github.com/chen-006/meow-llm-detector/tree/dc608d5c097abf96575d5b34f49952ddbd27c281) | [PolyForm Noncommercial 1.0.0](vendor/meow/LICENSE) |
| [ModelTrace — xqy2006](https://github.com/xqy2006/ModelTrace) | 原始指纹算法与统一候选库；[提交 6cd4a8f](https://github.com/xqy2006/ModelTrace/tree/6cd4a8ff0f3f4e49d6d6969dd2fbadfa1569fde2) | [MIT](vendor/modeltrace/LICENSE) |

宿主集成使用 [Codex Proxy 公开 SDK](https://github.com/zyycn/codex-proxy-rs/tree/c622f085748f812936a7371fcc85d1621fa72ca8/backend/crates/gateway-plugin/sdk)（v3.44.0，插件 RPC 协议 v7）、[@codex-proxy/ui](https://github.com/zyycn/codex-proxy-ui) v0.9.1，并参考 [官方插件示例与规范](https://github.com/zyycn/codex-proxy-plugins)。

插件 **v0.4.4 对应宿主 v3.44.0 / UI v0.9.1**，兼容下限仍为 **3.37.2**：SDK 新增代理目录与按代理 ID 出站接口，本插件继续使用原有模型执行与受管 HTTP，RPC v7 合同不变。UI 修复柔光材质联动圆角，保留宿主自定义尺寸与旧宿主弹层阴影回退。单题仍限时 120 秒；显式绑定账号不参与宿主队满换号，拒绝后保留发送状态：手动检测暂停等待人工继续，定时检测结束该账号本轮尝试并继续其他账号，不直连或自动重发。Key 可见模型仍不保证所选账号可用。差异及验证见 [SDK 适配分析](docs/sdk-upgrade-2026-11-11.md)。更旧宿主请保留匹配的旧插件。已发布制品以包内清单为准。

完整集成包按 [PolyForm Noncommercial 1.0.0](LICENSE) 分发，包含非商业用途条件；ModelTrace 等第三方文件仍保留各自的许可证。本仓库是源码公开项目，不将整个包宣称为 MIT 或无商业用途限制。原始版权声明、第三方组件声明与依赖许可证见 [NOTICE](NOTICE)，安装包中也附带相关文件。

## 安装与兼容性

| 项目 | 当前支持 |
| --- | --- |
| 插件 ID / 版本 | `local.model-health` / `0.4.4` |
| 宿主清单范围 | `>=3.37.2, <4.0.0`（插件 RPC 协议 v7、management v2、maintenance v1）；源码适配与离线协议验证基于 3.44.0 SDK，页面使用 UI v0.9.1 的 `z` 主题桥 |
| Release 平台 | Linux x86_64 与 ARM64（aarch64），glibc 2.36+（如 Debian 12）；不适用于 Alpine/musl 或 Windows |
| 运行方式 | `trustedProcess`，插件以宿主系统身份运行，安装代表完整信任；不是操作系统沙箱 |
| 扩展能力 | 独立管理页面与维护任务，支持手动检测、定时检测和飞书告警；不注册请求中间件、调度器或观察器 |

在 **插件管理 → 安装插件 → GitHub** 中填写：

```text
仓库：AlisaCat-S/codex-model-health-plugin
版本标签：留空查询本分支最新构建（v0.4.4-<UTC 时间戳>）
```

按服务器架构选择制品：AMD64/Intel 64 选 `local.model-health-<版本>-x86_64-unknown-linux-gnu.tar.gz`，ARM64 选 `local.model-health-<版本>-aarch64-unknown-linux-gnu.tar.gz`，核对平台与摘要后安装并启用。也可从 [Releases](https://github.com/AlisaCat-S/codex-model-health-plugin/releases) 下载对应文件，通过“上传包”安装。请不要上传 GitHub 自动生成的 **Source code (zip/tar.gz)**，它们是源码而不是插件制品。

版本号由 GitHub Actions 自动发布生成，格式为「语义版本 + UTC 时间戳」，如 `v0.4.4-20261111.1200`；分钟数不带前导零（05:04 记为 `20261024.504`）以符合 semver 预发布标识符规则，数值比较仍严格单调。详见 [docs/release-automation.md](docs/release-automation.md)。

Release 同时提供同名 `.tar.gz.sha256`；下载两个附件后可校验（以 x86_64 为例，aarch64 同理）：

```sh
sha256sum -c local.model-health-<版本>-x86_64-unknown-linux-gnu.tar.gz.sha256
# macOS：shasum -a 256 -c local.model-health-<版本>-x86_64-unknown-linux-gnu.tar.gz.sha256
```

保留 `local.model-health` 是为了让原先本地安装的版本继续沿用同一插件身份与历史。原项目为 [R-lmmortal/codex-model-health-plugin](https://github.com/R-lmmortal/codex-model-health-plugin)；本分支构建发布到 [AlisaCat-S/codex-model-health-plugin](https://github.com/AlisaCat-S/codex-model-health-plugin)。已有本地安装可在插件详情设置 GitHub 更新来源，或上传新版后确认切换；不要为了更新而删除原配置或私有数据。

## 使用

1. 按上述方式安装并启用插件。
2. 打开「账号模型体检」，选择已启用的 OpenAI 账号和已有 Key 的名称。账号优先显示去除首尾空白后的邮箱；没有邮箱时显示账号名称。新建检测保存该显示名称，旧历史仍保留检测当时的名称；请求绑定始终使用账号 ID。
3. 选择该 Key 可见的请求模型及检测方法。Meow 选择对照模型和 36 / 72 / 108 题档位；ModelTrace 使用统一候选库，固定 3 轮数字选择。切换方法保留账号、Key 和请求模型。
4. 点击「开始检测」，任务进入服务器后台队列。多个任务按开始或继续的顺序串行执行，宿主按所选 Key 计量；刷新、隐藏、关闭或离开页面不会暂停测试。
5. 手动新建的检测失败会暂停对应任务，其他排队任务继续。点「继续检测」才重新排队；每档最多追加 50% 重试预算。手动暂停或结束时，已发出的当前题仍会保存结果和用量。
6. 随时返回「任务列表」，点进任务查看进度、报告及请求明细；页面可见时每 2 秒更新状态。若旧任务显示「记录异常」，仍可查看请求明细并结束任务，再新建检测。任务列表支持每页 20 / 50 / 100 条和翻页，至少保留 1000 条完整历史报告及请求明细。执行槽位回收前先归档已结束且告警处理完成的记录，归档失败不会覆盖原记录。
7. **服务器或插件重启后，手动检测的未完成任务暂停，需手动继续；定时检测的暂停项结束本轮尝试并继续后续账号。** 中断题按一次未知结果计入尝试预算，已成功样本不重复发送。

Meow 报告给出行为匹配、行为偏移或证据不足。ModelTrace 报告展示前 5 个候选模型及家族概率，完整 18 个候选的结果保存在任务报告中；满 3 个有效样本才给出「更接近」结论。概率是在冻结候选库内、均匀先验下的相对分布，库外模型也会归到已有候选，不代表真实身份概率。

两种方法都不是智商分数或模型身份认证，分数不混用。仍只选择宿主 OpenAI 账号；ModelTrace 候选库包含 GPT / Claude 家族，不等于新增 Claude 原生账号接入。只有相同方法、基线、模型与采样条件的复测才适合比较。

## 定时检测与飞书告警

定时检测、报告和执行历史不依赖飞书配置；需要消息告警时，在「插件管理 → 设置」中填写飞书群机器人的 `feishuWebhook`，该地址作为敏感字段保存，不在插件页面或检测记录中回显
机器人可配置关键词「模型检测告警」或允许宿主出口 IP，当前发送接口不附带签名，开启签名校验的机器人无法接收消息

分组名称及成员统一通过宿主本机管理接口读取，与管理页保持一致，使用当前管理员会话且不保存会话 Cookie。宿主监听端口不是 8080 时，在插件设置将 `hostApiBaseUrl` 改为实际的 HTTP 回环 IP 与端口；默认 `http://127.0.0.1:8080`，不接受外部地址。分组读取失败会显示错误并允许重试，不把读取失败伪装成未分组。

在插件页面的「定时任务」中操作：

1. 点击「新建定时任务」，按分组多选检测账号：勾选组名选中该组全部可用账号，也可展开单独勾选；未加入任何分组的账号统一显示在「未分组」中（支持搜索、全选和清空），选择共用的计量 Key、测试模型、检测方法和预期模型，设置 1～43200 分钟的执行间隔
   多组共有的账号自动去重；保存的是本次明确选中的账号名单，之后分组成员变化不会自动修改已保存计划。编辑计划时会回显各组的已选数量，部分选中显示半选状态。
2. 点击「保存并启用」立即将首轮账号检测加入队列；「仅保存」保持停用，之后可单独启用。全部账号结束后再计算下次间隔，也可在等待期间点击「立即检测」提前执行
3. 「停用」停止后续调度，已经入队或执行的本轮继续完成，仍可能产生用量和告警；需要跳过单个账号时，点「查看本轮」进入报告后选择「跳过本账号」
4. 停用且本轮结束后可编辑或删除计划，已有报告保持不变；本轮未结束时重新启用会等待本轮完成，不重复创建检测
5. 列表显示整轮进度与下次执行时间，「查看本轮」逐账号查看状态、飞书投递结果和报告，「执行历史」按计划筛选全部保留历史并支持翻页；失败、暂停或重启中断的账号保留原因并结束本轮尝试，随后继续其他账号

ModelTrace 定时检测遇到数字不足（`insufficient_numbers`）时自动继续，3 个样本最多追加 2 次请求，成功样本不重复检测；预算耗尽后以「证据不足」结束该账号本轮，保留报告和失败明细，不发送模型不匹配告警，整轮结束后正常计时
定时检测遇到网络、鉴权、评分器故障或暂停时，等已发出的当前题保存结果后结束该账号本轮尝试，保留错误原因并继续后续账号，不自动重发失败请求；维护任务同样处理当前已暂停的定时检测。重启后已持久化且没有在途请求的排队账号继续执行，在途结果未知的账号保留中断记录后跳过。错误摘要保存在本轮列表中，完整报告过保留期后仍可查看原因；失败或跳过不发送模型不匹配告警。手动新建的检测仍可在报告中继续或结束。

最多保存 16 条定时任务，每条可选择全部已启用的 OpenAI 账号，选择计数显示已选数与可用账号总数；后台串行队列最多同时保留 16 个执行槽位；槽位回收前归档完整报告，历史至少保留 1000 条，本轮进度不依赖完整报告的保留期限
同一计划有未结束检测时不会创建另一轮；失败或暂停账号结束本轮尝试后参与整轮完成计数，启停配置与下次执行时间持久保存，进程重启后仍有效
错过多个周期只创建一轮，按本轮最后一个账号结束的时间重新计算间隔，不补跑积压轮次；单个账号无法创建检测时记录原因并继续其他账号
调度在逐题交接时检查，到期不保证立即发送请求；维护交接和其他检测排队可能延后执行

告警以完整完成的检测报告为依据：

- Meow 使用所选预期模型作为对照基线，仅在报告为「行为偏移」时告警
- ModelTrace 满足完整样本条件后，将报告第一候选的模型 ID 与预期模型比较，不一致则告警
- 测试模型使用别名时，预期模型可选择候选库中的对应模型，不通过模糊名称匹配自动合并模型
- 证据不足、请求失败、暂停或手动结束不作为模型不匹配告警

通知包含任务、账号显示名、测试模型、预期模型、结果模型、检测方法与记录 ID，不包含 Webhook、Key 或原始回答
每个账号各自判定告警，确认投递成功后不再发送，HTTP 成功且飞书业务码为 0 才视为成功
没有有效 Webhook 时，模型不匹配的报告仍然保留，告警标为「未配置飞书，已跳过」，不发送、不重试；以后配置飞书也不会补发已跳过的历史告警
失败或投递结果未知时，间隔至少 60 秒尝试，最多 3 次，达到上限后在列表显示告警失败；失败不重新执行模型检测
飞书接收消息后若响应丢失，或插件在保存成功状态前重启，重试可能造成重复通知，可用相同检测记录 ID 识别

告警依据行为指纹的相对结果，不代表模型身份认证

## 不匹配账号自动移组

定时检测的完整报告与预期模型不匹配时，后台将该账号**移出所有原分组，只保留「GPT降智分组」**，确认移组成功后从触发检测的定时计划后续账号名单中移除，并更新账号数量。其他定时计划不自动改写；本轮冻结名单、原始报告和请求明细保留。名单清空时自动停用该计划，重新选择账号后才能启用。

插件设置的 `hostAdminApiKey` 是宿主管理 API Key，作为敏感配置加密保存；仅通过已有 `hostApiBaseUrl` 的回环管理接口发送，不进入页面或报告。自动移组不依赖浏览器登录状态或飞书 Webhook，目标组必须已存在且名称唯一。未配置或失效时显示改组失败，不假装成功或减少账号数量。

Meow 仅在完整结束且报告为「行为偏移」时触发；ModelTrace 必须收齐全部有效样本且第一候选模型 ID 与预期模型不一致。请求失败、证据不足、暂停、手动结束及账号身份变化均不改组。该处理只针对带自动处理记录的新定时检测，不补处理升级前的历史报告，也不触发额外模型请求。

每个记录保存改组前分组 ID、处理状态与失败原因；成功后不重复处理。请求失败或响应丢失时至少间隔 60 秒重试，最多 3 次；重试先确认账号当前分组，已经只在目标组时不重复写入，继续完成计划更新。移组失败保留原计划名单；计划更新或处理状态写入失败会重试收尾。达到重试上限后保留失败状态并释放执行槽位，其他账号继续。可在「查看本轮」、报告和历史列表查看处理结果。

## 数据与执行

### 上游更新检查

右侧「项目来源」显示 [Meow](https://github.com/chen-006/meow-llm-detector) 和 [ModelTrace](https://github.com/xqy2006/ModelTrace) 的 GitHub 地址、内置提交与各自的「检查更新」按钮。内置来源来自 `vendor/meow-source.json` 和 `vendor/modeltrace/SOURCE.json`。宿主的隔离页面不开放新窗口，地址可点选或用旁边按钮复制，在浏览器打开；提交有差异时同样提供可复制的比较地址。

只有点击按钮才通过公开 SDK 的 `host.http.*` 读取对应仓库默认分支提交，并按 GitHub compare 的 ahead / behind / diverged / identical 区分结果。请求不使用账号、Key 或模型接口；打开页面不访问 GitHub，检查失败不影响开始、续测或检测历史。每个仓库的成功结果在插件进程内复用 5 分钟，展示实际检查时间；失败不缓存、不自动重试。每次检查限时 15 秒，单个响应最多 256 KiB。网络、限流或格式异常明确显示未确认，不当作最新版本。

检查比较的是上游仓库提交，不是可安装插件版本；文档等变化也会提示新提交。检测引擎、算法与基线随插件编译打包，保留「手动升级」：先适配并验证上游变更，再重新构建安装包，从插件管理更新同一插件。检查按钮不会下载或执行上游代码，不改正在运行任务的冻结基线与历史评分。

### 模型检测

- 使用 `host.auth.list/get_runtime` 的非秘密账号资料、`host.keys.list` 和 `host.models.list`；页面只提交账号 ID 与 Key ID，不读取原始凭据。
- 每个 `host.model.execute_stream` 都显式指定 `account_id`、`client_key_id`、`provider=openai` 和 `protocol=openai`。主服务负责代理、鉴权、配额与最终计量；插件不搭建另一套 HTTP 出口。
- 后台通过宿主 `maintenance`／`plugin.reconcile` 推进，使用维护调用自身的宿主客户端。代理、上游请求头及客户端参数按宿主有效配置应用；没有浏览器入口的后台调用不伪造入口 IP 或 User-Agent。宿主拒绝或代理失败时保留原因：手动检测暂停，定时检测结束该账号本轮尝试并继续其他账号，不回退为插件直连上游。
- 维护调用声明 1 小时有限预算，单题总执行限时 120 秒，模型流读取仍限时 90 秒。剩余不足 150 秒时不派发下一题，交回宿主后继续队列，轮次交接最多等待约 30 秒。持久化失败时停止继续派发；恢复后手动检测保持暂停，定时检测结束在途结果未知的账号并继续已持久化的排队任务。
- 网页在开始及继续时显式传入 `background:true`。兼容旧调用者：`runs/start` 省略该字段仍创建手动逐题任务，原 `runs/step` 可用；后台托管任务拒绝逐题调用。旧记录在新页面点击继续后转入后台队列。任务新增可选的 `background`、`queued_at`、`task_error`，沿用原状态枚举与存储 schemaVersion 1。
- 支持宿主返回的 Responses SSE 与 WebSocket 事件，统一交给原解析器判定完成、错误与拒答；读至宿主 EOF 后才评分。非流式错误只显示载荷类型及可用的 HTTP 状态/传输方式，不保存原始错误正文；用请求 ID 关联宿主详情。
- 完整保留 Meow 的题目、参数、SSE 解析、答案归一化和概率评分。原源码放在 `vendor/meow`，文件哈希没有改变。Meow 版本 4.5.4；基线内容哈希冻结在每次任务中。
- ModelTrace 的原始 `fingerprint.py` 和统一候选库固定在上游提交 `6cd4a8ff0f3f4e49d6d6969dd2fbadfa1569fde2`，来源及逐文件哈希见 `vendor/modeltrace/SOURCE.json`。复用原始随机题目生成、数字解析、Hellinger / 有序数字块融合、温度校准和家族聚合，不引入原项目的网络服务或 API Key 表单。
- ModelTrace 生成 3 份独立题目后持久化，断点续测与失败重试保留题目和检测方法。通过现有 Responses 流发送单条用户消息，采样使用供应商默认值，不套用 Meow 的 256-token 短答参数。沿用插件模型流读取阶段 90 秒上限（不含此前的请求建立）、后台串行执行、遇错人工续测和 50% 重试预算（3 个有效样本、最多 5 次尝试），而非原网页的自动重试流程。
- 后台任务以带排队时间的 `ready` 表示排队，逐题 `ready → running → ready/paused/completed`；显式停止转 `stopped`。发送前先写 pending，异常中断后按一次未知结果计入预算。首轮所有题优先于重试；成功样本不重复计数。
- 全局推理并发为 1；网络等待时释放状态锁。页面暂停/结束保留当前题的结果。每题发送前及续测时核对账号 ID、平台、启用状态和冻结的上游身份；Cookie 回写、令牌刷新导致的凭据版本变化不视作换号，认证类型或上游身份变化则中止续测。
- 缺少稳定上游身份的账号，在已有样本且凭据更新时要求新建检测。旧检测没有身份快照时，只有尚无样本或凭据版本未变才会补存当前身份；其他旧检测保留历史，结束后新建一次即可使用稳定身份校验。
- 检测记录先写入插件私有 `runs` 命名空间，16 个执行槽位，CAS 更新；回收槽位前将完整记录以 gzip / Base64 归档到 14 个 `history-*` 命名空间，每个 72 条，总共 1008 条归档，另保留当前 16 个槽位。只在归档容量已满时按创建时间淘汰最早归档；原有仍保留的记录继续可读，已经被旧版覆盖的历史无法恢复。分页接口为 `POST api/runs`，参数 `page`、`page_size`（20/50/100）、可选 `schedule_id`，返回 `runs`、`total`、`page` 和 `page_size`；首次读取重建轻量索引，后续仅读取当前页报告。定时计划写入 `schedules` 命名空间，飞书告警状态随所属检测记录保存；未处理完告警的记录不参与槽位替换，不改主服务数据库结构，不安装观察器或中间件。
- 样本保留请求 ID、用量、错误与最多 256 字节回答摘录。Meow 保存归一化类别；ModelTrace 保存完整有序数字（最多 4096 个，超出则明确报错），用原评分器的有效长度判定剔除短答。评分不使用展示摘录，展示截短会标明。
- 历史报告保留检测方法、基线版本和内容哈希，不因表单切换或后续基线更新重新计算。缺少方法字段的旧记录按 Meow 读取和续测，不改变原来的账号身份校验。
- 主服务的模型映射、提示词或中间件可能影响实际响应，因此报告描述的是当前配置链路的行为，不把它解释成纯模型能力证明。
- Linux 安装包内含 Rust 插件及 Python/NumPy 检测引擎，不要求服务器另装 Python。引擎在宿主分配的插件实例目录内创建权限 0700 的工作目录，并将子进程的 `TMPDIR` 指向这里；支持非 root、空环境、只读根目录及 `/tmp:noexec`。宿主回收实例目录时会清理残留文件；不是 Alpine/musl 包。

## 构建与验证

构建工具：Docker Buildx、Node 24+、pnpm 12.14、Python 3.9+（安装包冒烟测试）、Rust 1.99.1（构建打包 CLI）。SDK 与 CLI 固定为同一个宿主提交 `c622f085748f812936a7371fcc85d1621fa72ca8`（v3.44.0，插件 RPC 协议 v7）。UI 使用官方固定版本 Release 归档，不再从 Git 源码安装或执行其构建脚本。前端依赖锁定在 `frontend/pnpm-lock.yaml`；不要直接换成浮动 `main`。在仓库根目录执行：

```sh
cargo install --git https://github.com/zyycn/codex-proxy-rs.git \
  --rev c622f085748f812936a7371fcc85d1621fa72ca8 \
  --locked --root .tools/cpr-cli codex-proxy-plugin-cli
CPR_PLUGIN_BIN="$PWD/.tools/cpr-cli/bin/cpr-plugin" ./scripts/build-linux.sh
pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend test
pnpm --dir frontend build
```

`./scripts/build-linux.sh [目标]`（默认 `x86_64-unknown-linux-gnu`，可选 `aarch64-unknown-linux-gnu`）在 Docker 中构建内嵌 Python/NumPy 引擎、运行 Rust 的 `bundled-engine` 单测、编译对应架构的 Linux 二进制、构建页面，最后使用官方 CLI 生成 `dist/*.tar.gz` 与校验文件。打包后在非 root、空环境、只读根目录和 `/tmp:noexec` 容器中运行离线协议验收；默认使用 `debian:bookworm-slim`，可通过 `SMOKE_IMAGE` 指定已准备好的 Debian 12 兼容镜像，`SMOKE_PLATFORM` 默认与目标架构一致。

### 自动构建发布

`.github/workflows/release.yml`：推送 `main` 自动构建并发布 Release；也可在 Actions 页面手动触发（`workflow_dispatch`），勾选 `dry_run` 可只构建验收不发布，便于在分支试跑。每次运行用原生 runner（`ubuntu-latest` / `ubuntu-24.04-arm`）同时构建 x86_64 与 aarch64 两种制品，版本号自动生成为「语义版本 + UTC 时间戳」（如 `v0.4.4-20261111.1200`），并附同名 `.sha256`。设计与取舍见 [docs/release-automation.md](docs/release-automation.md)。

直接运行不带 `bundled-engine` 的本机 `cargo test` 默认使用开发用 `meow-llm-detector:4.5.4` Docker 镜像。若本机已有 Python 和 NumPy，可显式指定 `MODEL_HEALTH_TEST_PYTHON` 为 Python 可执行文件路径，不会根据 Docker 是否安装自动切换环境。例如 PowerShell：

```powershell
$env:MODEL_HEALTH_TEST_PYTHON='C:/Python313/python.exe'
cargo test --locked --manifest-path backend/Cargo.toml
python -X utf8 -B -m unittest discover -s scripts/tests -v
```

发布构建仍使用内嵌引擎。构建脚本同时校验 Python 冒烟编码与 Rust SDK 的 RPC v7、GPE2/HME1 格式；安装包冒烟覆盖挂起/恢复授权/回调消费、SSE、两种 WebSocket 封套、多批次和独立 EOF。修复设计与验收范围见 [Responses 流修复方案](docs/investigation/2026-10-24-arm-detection-failures.md)。

浏览器预览显式标注「模拟」，生产构建会剔除模拟宿主与模拟数据模块。`scripts/browser-qa.mjs` 支持 `PLAYWRIGHT_PATH`、`CHROME_BINARY`、`PREVIEW_URL`，检查双方法切换、选择保留、报告隔离、开始→暂停→继续→结束→历史，以及浅/深/窄屏、空/错/加载状态。`scripts/background-browser-qa.mjs` 使用独立于页面的模拟宿主，验证刷新、关闭、离开后重新进入任务、轮询断连恢复及旧详情响应不会覆盖新选择。`scripts/updates-browser-qa.mjs` 使用相同环境变量，验证来源地址、更新检查状态和检查失败时模型表单仍可使用，并在 `sandbox="allow-scripts"` 的隔离页面验证生产资源与地址复制。

`scripts/schedules-browser-qa.mjs` 使用相同浏览器环境变量，验证整组选择、未分组归类、重叠分组去重、半选状态、搜索与读取失败重试，以及 47 个账号的全选、清空、保存、重新编辑和空列表；后台协议测试覆盖执行槽位轮换后，全部 47 个账号仍各完成一轮且不重复入队。

`scripts/remediation-browser-qa.mjs` 验证自动移组状态、后续账号数量更新、冻结的本轮进度和空计划停用。

`scripts/history-browser-qa.mjs` 验证 1024 条记录的翻页、每页条数、计划筛选、跨页报告和迟到响应；后台测试覆盖 1008 条归档、重启读取、容量淘汰、写入失败保护以及索引重复利用。

`python3 scripts/package-smoke.py /absolute/path/plugin.tar.gz --continuous --modeltrace --updates` 在离线 SDK 宿主夹具中检查安装包、两种检测、更新回调和互不干扰。另加 `--updates-live` 时仅由测试宿主访问两个公开 GitHub API（不调用真实模型），验证安装包实际解析和缓存上游提交；网络失败会让该项验收失败。

离线协议验收使用固定模拟回调，不向真实模型发送请求。安装后的最终验收是用一个账号分别执行 Meow 快速档和 ModelTrace 三轮检测，核对主项目使用记录中的账号与请求 ID。安装与真实计费检测由使用者主动触发。

## 回退

宿主 v3.44.0 仅接受 RPC v7，页面主题使用 `z` 命名空间，与 v3.37.2 一致。更旧插件即使同为 RPC v7，也可能依赖旧版 `cp` 主题桥；回退须选择主题与协议均匹配的宿主和插件制品，不能只核对 RPC 版本。

安装新版时在插件管理中更新同一插件，保留现有 `runs` 状态。升级前保留旧安装包并备份宿主数据；需要回退时使用插件详情中的“回退版本”或“切换版本”，选择已安装、已接受且兼容的旧制品。配置回退不等于恢复私有数据，不能用卸载重装代替历史备份。停用本插件会停止插件功能，不修改主项目代码或其他插件。

## 后续开发提示词

```text
继续开发 codex-model-health-plugin。先读 README.md、plugin.json、engine/worker.py、backend/src 和现有协议测试。
沿用 @codex-proxy/ui 与当前管理桥；保持 Meow / ModelTrace 原源码不变、方法与基线冻结、旧 Meow 记录兼容、账号/Key 显式绑定、预算保守计数、错误不参与评分。
只改所请求的功能；补充离线协议测试和真实浏览器交互检查；不要自动部署或执行付费检测。
```
