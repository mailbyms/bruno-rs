# Bruno RS

用 Rust + Tauri 2 重写 Bruno API 客户端：保留 Bruno 的 React 前端，用 Rust 实现全部后端逻辑。
**仅支持 OpenCollection YAML 集合格式**（不读 `.bru` / `bruno.json`）。

## 里程碑 1：打开集合 → 发请求 → 看响应（已完成）

已在 Windows 上跑通完整 UI 闭环：

- 启动即恢复上次工作区的集合，侧栏自动展开集合树（启动落在「工作区 Overview」，含快速操作）
- 点击请求 → 打开标签页（Params/Body/Headers/Auth/Vars/Script/Assert/Tests 全部可用）
- 点 Send → 响应面板显示状态码、耗时、大小与格式化 body，断言勾选状态同步
- 嵌套文件夹、JSON body、声明式断言、环境变量插值均按 OpenCollection YAML 语义工作

## 里程碑 2：保存 / 脚本 / 认证（已完成）

- **保存请求落盘**：`save_request` 不再手拼 YAML，改为「读原文件 → 合并 info/http/runtime/settings → 原子写回」。
  未知顶层键、`docs`、未编辑的 settings 键都保留；body 覆盖 json/text/xml/sparql、
  form-urlencoded、multipart-form、file。保存后立即重解析并同步内存索引（`load_request` 返回新值）。
- **QuickJS 脚本**（`rquickjs`）：`runtime.scripts` 的 before-request / after-response 与 `tests` 全部执行。
  支持 `bru.*`、`req.*`、`res.*`、`test()`、chai 风格 `expect`（含 `.not`）、`console.*`；
  `bru.setVar` 增量通过 `runtimeVariables` 回传并合并到 collection，跨请求可用。
- **认证**：digest（RFC 7616 MD5，401 挑战后重试）、OAuth2（client_credentials / password / refresh_token，
  token 走 header 或 query）、AWS SigV4、WSSE；basic/bearer/apikey 继续可用。UI 的「获取 Token」按钮已接
  `renderer:fetch-oauth2-credentials` / `refresh-oauth2-credentials`。
- **请求体**：multipart-form（text/file，文件路径相对集合目录解析）与 file/binary。
- **UI 结果**：Tests 页签显示 tests / assertions / before-request / after-response 四组结果，页签角标为总数。

示例集合 `fixtures/scripted`：`scripted.yml`（before-request 改写 URL/header + 3 条 tests + 1 条断言）、
`upload.yml`（multipart 表单 + `{{scriptTag}}` 跨请求变量）。

## 里程碑 3：CRUD / 环境 / Runner / 导出 / Git（已完成）

按「bruno vs postman 差距清单」分三批推进后的结果：第一批（CRUD 与设置）、第二批（Runner / 导出 /
Cookie / 文件监听 / 快照）、第三批只做 Git（Mock Server 与 OpenAPI 导入不做）。

- **请求 / 文件夹 / 集合 CRUD**（`commands/fsops.rs` + `crates/bruno-core/src/ops.rs`）
  - 新建请求（重名自动 `-2` 后缀）、新建文件夹、删除、拖拽移动、改显示名、改文件名、重排序
  - 新建 / 克隆集合与文件夹、集合根「请求默认值」、文件夹根设置（`folder.yml`）
  - 每个写操作后整份重载集合再推 `main:collection-tree-loaded`；**uid 按路径稳定生成**
    （`ops::stable_uid`），前端 `mergeTreeItems` 才能保住草稿与已打开标签页
- **环境**（`commands/env.rs` + `envs.rs`）：集合环境（`<collection>/environments/*.yml`）、
  工作区环境、全局环境（数据目录 `global-environments.json`）的增删改查 / 改名 / 改色 / 继承 / 选中；
  全局环境按「按 uid 合并」写入，所以「保存变量」不会把 name/color 冲掉
- **偏好设置**：`preferences.json` 深合并持久化，`request.timeout` / `sslVerification` / `proxy`
  直接作用于真实请求（`state::request_defaults()` → `RequestOptions`）
- **集合 Runner**（`runner_batch.rs` + `commands/run.rs`）：按 `main:run-folder-event` 流式推送
  started / queued / request-sent / response-received / test-results / assertion-results / error / ended，
  支持递归、tag 与单个 uid 过滤、请求间隔、取消（`skipped` 计入被过滤与取消的请求）；
  每个请求与单请求共用同一套偏好、代理与 **cookie**（见下）
- **Cookie**（`cookies.rs`）：自实现 CookieJar（reqwest 的 cookie_store 无法枚举），
  解析 `Set-Cookie`、按 domain/path/secure 匹配、跨请求自动带上；Cookies 面板的
  增删改 / raw 字符串互转 / 清空域名全部可用（面板数据按域名分组，`expires` 为毫秒）
- **导出**：集合 zip（`zip` crate，跳过 `.git`/`node_modules`）、集合 YAML、Postman
  collection v2.1、环境 JSON、工作区导出
- **文件监听**（`watcher.rs`）：外部编辑器改动 `*.yml` 后 0.7s 内自动重载并刷新侧栏。
  不引入 `notify`，用「目录指纹（文件名 + mtime + size）轮询」实现，只关心
  `yml/yaml/json/bru` 与 `.env*`，忽略 `node_modules`/`.git`/`dist`/`target`
- **标签页快照**：`snapshot:save/get/clear/get-tabs` 落到数据目录，供下次启动恢复
- **Git**（`git.rs` + `commands/git.rs`）：直接调系统 `git`（Windows 不弹黑框），
  克隆仓库、列远端分支、读集合 remote、连接 / 断开 remote（写入工作区配置）。
  `is_valid_url` 拒绝 `-` 开头的参数注入，并允许 `file://` 与本地绝对路径
- **窗口控制**：最小化 / 最大化 / 全屏 / 关闭 / 缩放 / 开发者工具

一条命令覆盖上述全部通道：`node scripts/verify-batch12.mjs`（46 项断言，走真实 `ipcRenderer`）。

## 里程碑 4：Git 侧栏、UI 恢复、工作区（已完成）

在上面的基础上补齐「用户点得到但点了没用」的那批入口（OpenAPI 同步与 AI 明确不做）。

- **Git 侧栏入口**（`Sidebar/Collections/Collection/CollectionRow`）：集合右键菜单里
  「Connect to Git」/「Update Git Remote」，已连接时额外出现「Disconnect Git Remote」。
  只对属于当前工作区的集合显示（`remote` 存在 `workspace.yml` 的集合条目里）；
  复用 `ConnectGitRemote` / `RemoveGitRemote` 弹窗，成功后由 `main:workspace-config-updated` 刷新菜单文案
- **UI 快照真正接上**：新增 `snapshot_get_collection` / `snapshot_get_sidebar` /
  `snapshot_update_ui_state`。挂载集合时恢复上次选中的环境，启动时恢复侧栏宽度/折叠，
  切换环境时按 patch 回写；标签页恢复沿用 `snapshot_get_tabs`（前端 middleware 负责整份序列化）
- **集合设置落盘**：`update-bruno-config`（版本号等）、`save-collection-security-config`
  （写 `opencollection.yml` 的 `settings.security`，读回同样走该键）；`get-collection-workspaces` /
  `set-collection-workspace` 让「从工作区移除」不再误判断集合是否还被别的工作区引用
- **工作区（多工作区）**：`commands/workspace.rs` + `Settings.workspaces`
  - 每个工作区一个目录，`workspace.yml` 记录 `name` / `collections` / `docs`；
    uid = `stable_uid(path)`，集合条目里可带 `remote`
  - 创建 / 打开目录 / 打开对话框 / 改名 / 关闭（只移出列表，不删目录）/ 保存文档 /
    导入工作区 zip / 导入工作区环境 / `find-unique-folder-name` / `ensure-collections-folder`
  - 默认工作区行为**完全不变**（集合仍存 `settings.json`，`active_workspace_uid` 为空即默认工作区）
  - 启动时逐个回推 `main:workspace-opened`（当前激活那个 `type` 置为 `default` 以便前端切过去），
    再发 `main:workspaces-ready`，由快照里的 `activeWorkspacePath` 决定落到哪个工作区
- **ZIP 导入**：`is-bruno-collection-zip`（识别 `opencollection.yml` / `bruno.json` / `collection.bru`）、
  `import-collection-zip`（解压到 `<location>/<包名>`，自动剥掉 zip 内多余的一层目录，防 zip slip）
- **其它小通道**：`get-system-proxy-variables`（环境变量 + Windows 注册表代理）、
  `refresh-system-proxy` / `refresh-pac-cache` / `clear-http-https-agent-cache`、
  `load-large-request` / `load-request-via-worker`（本实现挂载时已全量读入，直接返回请求内容）、
  `theme-change`（把主题底色同步给窗口，减少启动白闪）

验证脚本：`node scripts/verify-batch3.mjs`（34 项，见下）。

## 里程碑 5：WebSocket 与 gRPC（已完成）

Bruno 的 WS / gRPC 面板原本只有前端，后端通道全返回 null。本轮把两条长尾子系统接上，
并让这两类请求能出现在侧栏、能编辑保存、能真正收发。

- **WebSocket**（`crates/bruno-core/src/ws.rs` + `src-tauri/src/commands/ws.rs` + `hubs.rs`）
  - `renderer:ws:start-connection` / `queue-message` / `send-message` / `close-connection` /
    `connection-status` / `is-connection-active` 全部实现
  - 事件严格对齐 `utils/network/ws-event-listeners.js`：connecting / upgrade / request / open /
    message（incoming+outgoing）/ close / error / disconnecting，payload 一律
    `[requestId, collectionUid, eventData]`
  - `options.connectOnly` 为假时按原版语义把**所有内容非空的消息**排队；单条消息走
    `queue-message`（按 `selectedMessageIndex` 取）。「连接中」阶段消息进无界通道，
    open 之后顺序发出，等价于原版的队列
  - 支持 `settings.timeout`（毫秒）、`settings.keepAliveInterval`（Ping 保活）、
    headers 合并 + basic/bearer/apikey/wsse/oauth2 认证。消息类型沿用前端 `text` / `json` / `xml`；
    若收到 `type: binary` 会按 base64 解码成二进制帧（UI 目前不产生该类型，属防御分支），
    反方向的二进制帧按 UTF-8 宽松解码 + `messageHexdump` 显示
- **gRPC**（`crates/bruno-core/src/grpc.rs` + `grpc_proto.rs` / `grpc_codec.rs` /
  `grpc_reflection.rs` + `commands/grpc.rs`）
  - `grpc:start-connection` / `send-message` / `cancel` / `end` / `cancel-request` /
    `end-request` / `is-connection-active` / `load-methods-proto` / `load-methods-reflection` /
    `generate-sample-message` / `generate-grpcurl`
  - 四种方法都支持（unary / server-streaming / client-streaming / bidi）；描述符来源二选一：
    本地 `.proto`（`protox` 纯 Rust 编译，发布包不需要 protoc）或 **server reflection**
  - 方法列表带 `requestFields` / `sampleRequest`，「重新生成示例消息」不需要缓存描述符；
    `generate-sample-message` 会保留用户已填的值、只补缺失键
  - 事件对齐 `grpc-event-listeners.js`：request / message / response / metadata / status /
    error / server-end-stream，`requestId` 就是 `item.uid`
- **落盘格式**：WS / gRPC 请求在磁盘上是独立段（`websocket:` / `grpc:`），
  `info.type` 分别是 `websocket` / `grpc`；`GrpcDetails` 用 `methodType` / `protoFilePath` /
  `metadata`，消息单条写字符串、多条写变体数组。重新加载时按类型产出前端形状
  （`request.method` / `methodType` / `protoPath`、`body.ws` / `body.grpc`）
- **连接注册表**（`hubs.rs`）：`WsHub` / `GrpcHub` 记 requestId → 连接，
  供 cancel/end/status 用；同一 requestId 重连会先关旧连接，并用**自增代号**保证旧任务的
  `on_exit` 不会把新连接从表里摘掉

示例集合 `fixtures/protocols`：`ws-echo.yml`（两条消息，第一条 selected）、
`grpc-echo.yml`（unary，相对 `echo.proto`）、`echo.proto`（4 个方法覆盖四种流式类型）。

### 已实现

- **Rust 核心库**（`crates/bruno-core`）
  - OpenCollection YAML 数据模型（对齐 `@opencollection/types@0.15.0`，宽容解析）
  - 集合加载：递归扫描 `opencollection.yml` + `*.yml` + `folder.yml` + `environments/*.yml`
  - 变量插值（`{{var}}`，优先级 global < collection < environment < folder < request < runtime）
  - 声明式断言（`res.status` / `res.body.x` 路径 + 28 个操作符）
  - HTTP 执行引擎（reqwest + rustls，脚本 → 插值 → 认证 → 发送，digest 挑战重试）
  - `persist.rs`：保存请求时按映射合并回 YAML，保留未知键；`scripts.rs`：QuickJS 沙箱（64MB 内存上限）
  - `postman.rs` + `import.rs`：Postman Collection v2.0/2.1 → Bruno 内存集合 → OpenCollection YAML 目录
- **Tauri 2 后端**（`src-tauri`）
  - 与 Bruno 原版一致的 IPC 通道名，前端通过 `window.ipcRenderer` shim 零改动复用
  - 集合挂载 + 树事件一次性推送（`main:collection-opened` / `main:collection-tree-loaded`）
  - 默认工作区持久化（`settings.json` 的 `workspace_collections`），重启后侧栏自动恢复
  - 请求执行、文件保存、目录选择、最近集合、Scratch 集合目录
  - 单一路径入口 `paths.rs`，`BRUNO_RS_DATA_DIR` 可覆盖
- **前端**（`web/`，从 bruno-app 复制）
  - `src/ipc-shim.js`：Electron IPC 契约 → Tauri commands / events，未实现通道返回类型正确的空值
  - 保留全部 React 19 + Redux Toolkit + styled-components + CodeMirror UI

### Postman 导入

侧栏「Import Collection」→ 选 Postman 导出的 `.json` → 填目标目录 → Import。走完整 UI，
不需要命令行。后端做两件事：

1. `renderer:convert-postman-to-bruno`（`crates/bruno-core/src/postman.rs`）
   - item 树、header、query / path 参数（query 从 raw URL 里拆出来，避免发送时重复）
   - body：raw(json/xml/text)、urlencoded、formdata(text/file)、binary file、graphql（转 JSON 文本）
   - auth：basic / bearer / apikey / digest / awsv4 / ntlm / oauth2(client_credentials 映射)
   - 脚本：`prerequest` → before-request、`test` → tests，`pm.*` 做常见文本映射
     （`pm.response.code` → `res.getStatus()`、`pm.expect` → `expect`、`pm.environment.set` → `bru.setVar` 等）
   - 无法映射的项进 `issues`，前端用现有 toast 提示
2. `renderer:import-collection`（`crates/bruno-core/src/import.rs`）
   - 在目标目录下建 `<集合名>/`，写 `opencollection.yml`、`folder.yml`、每个请求一个 `.yml`
   - 同名目录/请求自动加 `-2` 后缀；文件名只替换 Windows 非法字符，保留中文
   - 导入后立即挂载并推送 `main:collection-tree-loaded` + `main:workspace-config-updated`，侧栏直接出现

只支持导入成 YAML（`options.format` 被忽略）；Postman 集合里的 `pm.*` 复杂脚本仍需要人工调整。

### 运行

```bash
# 依赖
cd web && npm install && npm run build && cd ..
cargo build --features custom-protocol

# 开发模式（前端 dev server + Tauri）
npm run dev:app

# 构建发布 exe（release 必须带 custom-protocol，否则前端加载 localhost:1422）
npm run build:app
```

启动后可用「打开集合」选择目录，或命令行直接传路径：

```bash
target/debug/bruno-rs.exe "C:/Users/DM/Documents/workspace/github/bruno-rs/fixtures/shop"
```

示例集合：`fixtures/demo`（最小 GET）、`fixtures/shop`（嵌套文件夹 + POST JSON body）、
`fixtures/scripted`（脚本 / 断言 / multipart）、`fixtures/protocols`（WebSocket + gRPC）。

### 本地验证

```bash
node scripts/mock-server.mjs          # 127.0.0.1:18081 回显服务，不依赖外网
node scripts/ws-echo-server.mjs       # 127.0.0.1:18082 WebSocket echo（零依赖）

# 隔离数据目录 + CDP 启动应用
BRUNO_RS_DATA_DIR=/tmp/brs-data \
WEBVIEW2_USER_DATA_FOLDER=/tmp/brs-wv \
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9236 \
./target/debug/bruno-rs.exe fixtures/shop

node scripts/smoke.mjs                # UI 冒烟：侧栏树 → Send → 断言 200 + {"ok": true}
node scripts/verify-batch12.mjs       # 里程碑 3 全通道：CRUD / 环境 / 偏好 / Cookie / Runner / 导出 / 快照 / Git
node scripts/verify-batch3.mjs        # 里程碑 4：集合设置 / UI 快照 / ZIP 导入 / 工作区 / 长尾通道
node scripts/verify-m2.mjs            # 里程碑 2：脚本 → Tests 角标 → 保存/重载 → multipart
node scripts/verify-ws-grpc.mjs       # 里程碑 5：WS 连接/收发/断开 + gRPC 方法枚举/示例消息/错误事件（CDP 9244）
node scripts/verify-import.mjs        # Postman 导入（IPC 链路：转换 → 落盘 → 发请求）
node scripts/verify-import-ui.mjs     # Postman 导入（真实 UI：菜单 → 选文件 → Import → 发请求）
node scripts/probe-startup.mjs        # 启动期控制台错误（应为空）
cargo test -p bruno-core              # 53 个测试（36 单测 + 6 核心 + 5 集成 + 6 WS/gRPC）
```

`verify-ws-grpc.mjs`（27 项断言）前置：`node scripts/ws-echo-server.mjs` +
应用以 `--remote-debugging-port=9244` 启动（不传集合也可，脚本自己 `open-collection`
`fixtures/protocols`）。gRPC 侧不需要真实服务：用 proto 校验方法枚举与示例消息，
用不可达端口（`127.0.0.1:18083`）校验 `grpc:error` 的 code 14 与 `grpc:server-end-stream`。

`verify-m2.mjs` 会临时改写 `fixtures/scripted/scripted.yml` 的 URL 并在结束时还原；
它要求 `fixtures/scripted` 已经在侧栏打开（应用启动时用参数传入，或先调一次 `open-collection`）。
`verify-import*.mjs` 用系统临时目录做导入目标（可用 `IMPORT_LOCATION` 指定），不会污染 `fixtures/`。
`verify-batch12.mjs` 在系统临时目录里新建集合，改完即弃；它需要 mock server 与 CDP 实例
（`CDP_PORT` 默认 9240，`MOCK_URL` 默认 `http://127.0.0.1:18081`）。
`verify-batch3.mjs` 同样只用临时目录；它会临时改写 `fixtures/shop/opencollection.yml`（版本号与
`settings.security`）并在结束时还原。

除 `verify-ws-grpc.mjs`（9244）外，其余脚本都读 `CDP_PORT` 环境变量，可以指到同一个已启动实例上串跑，
例如 `CDP_PORT=9236 node scripts/verify-batch3.mjs`。

辅助脚本：`probe-state.mjs`（读 Redux）、`probe-events.mjs`（事件桥）、`click-row.mjs`（按文本点侧栏行）、
`probe-errors.mjs`（异常栈）、`diag-events.mjs`（事件诊断）。

### 应用数据目录

- Windows：`%APPDATA%/bruno-rs/`
- macOS：`~/Library/Application Support/bruno-rs/`
- Linux：`$XDG_DATA_HOME/bruno-rs/`
- 可用 `BRUNO_RS_DATA_DIR` 覆盖（隔离测试用）

### 实现要点（改代码前必读）

- **叶子请求不能带 `items` 键，文件夹不能带 `request` 键**。Bruno 前端判定：
  `isItemARequest = hasOwnProperty('request') && !items`、`isItemAFolder = !hasOwnProperty('request') && type === 'folder'`。
  所以 `TreeNode` 的 `items` / `request` / `settings` 都要 `skip_serializing_if`，文件夹默认值放 `root.request`。
- **`body.<mode>` 必须是字符串**（`json`/`text`/`xml`/`sparql`），前端直接把它当 CodeMirror 的 `value`；
  传对象会让 CodeMirror 把它当成 Doc 实例，报 `Cannot add property modeOption, object is not extensible`。
- **Windows `canonicalize()` 会加 `\\?\` 前缀**，必须剥掉再和前端比较路径，否则工作区匹配失败。
- **启动事件握手**：`renderer:ready` 只发 preferences；前端在 `useIpcEvents` 里注册完所有 `main:*` 监听后，
  下一个宏任务调 `renderer:app-ready`，后端此时才回推 `workspaces-ready` / `app-loaded` / `workspace-opened`。
  不要在后端写固定 sleep。
- **树事件可能早于集合进 Redux**：前端在 `main:collection-tree-loaded` 里按 `collectionUid` 轮询等待集合就位，
  就位后一次性灌树、标记 mounted、展开集合。不要在这里抢标签页焦点——`switchWorkspace` 随后会加
  Overview/Environments 工作区标签，抢焦点会变成竞态。
- **`web/dist` 改动后必须重新 `cargo build --features custom-protocol`**，exe 内嵌的是构建时的 dist。
- **UI auth 是嵌套形状** `{mode, [mode]: {...}}`，磁盘是 `{type, ...}`（inherit 是纯字符串）。
  唯一转换入口是 `persist::disk_auth_to_ui` / `persist::ui_auth_to_disk`；写成扁平形状会让认证表单空白、
  请求不带认证头（`apikey.placement` 还要在 `query` ↔ `queryparams` 之间映射）。
- **body 的磁盘 type ≠ UI mode**：`form-urlencoded`、`multipart-form`、`file`；写回时必须由
  `ui_body_to_disk` 映射，读入时由 `body_to_ui` 反向映射，否则保存一次就把 multipart 清空。
- **树节点的 uid 必须按路径稳定生成**（`ops::stable_uid`）。写成随机 uid 时，每次树刷新（含 CRUD、
  文件监听）都会让前端 `mergeTreeItems` 认不出旧节点，草稿和已打开标签页一起丢。
- **同一把 `Mutex` 不要持锁再调 `save_*`**：`save_cookies()` / `save_settings()` 内部会再锁一次，
  `std::sync::Mutex` 不可重入，表现为该命令永不返回（曾经让整个应用卡死）。
  需要落盘时先 `drop(guard)` 或把锁放进独立 `{}` 作用域。
- **`main:run-folder-event` 必须出现在 shim 的 `BRIDGED_EVENTS` 里**。它是 Runner 的唯一进度来源，
  漏桥接的表现是「Runner 跑完了但结果面板一片空白」。
- **`main:git-version` 这类启动事件要在 `app_ready` 里补推一次**：`renderer:ready` 阶段可能早于
  前端 `listen()` 注册完成，只发一次会丢。
- **Cookies 面板的数据形状是「按域名分组」**：`[{domain, cookies:[{key, value, path, expires(ms), ...}]}]`，
  字段名是 `key` 不是 `name`，`expires` 是毫秒时间戳。分组在 Rust 侧完成，前端只渲染。
- **保存只合并、不整份重写**：只覆盖 `info/http/runtime/settings`，保留未知键；settings 只回写非空且
  camelCase 的键，避免产生 `encode_url: null`。YAML 注释无法保留（serde_yml 无注释模型）。
- **脚本结果不进 response**：`sendNetworkRequest` 只透传固定字段。新增 `testResults` /
  `preRequestTestResults` / `postResponseTestResults` / `assertionResults` 时必须同步改
  `utils/network/index.js` 与 `responseReceived` reducer，否则后端算完也到不了 UI。
- **断言结果有两个消费方**：Tests 页签读 `status/lhsExpr/rhsExpr`，Assert 页签读 `passed`，两者都要输出。
- **集合变量有两种形状**：`collection.root.vars.req`（内存集合）与 `collection.root.request.vars.req`
  （`main:collection-tree-loaded` 的 payload）。`runner::collect_vars` 两种都要读，只读一种会让
  `{{baseUrl}}` 这类集合变量永远不插值（导入的 Postman 集合因此全挂）。
- **导入必须复用 `persist::save_request_file`**：它同时负责 body/auth/runtime/settings 的映射；
  自写 YAML 拼接会漏掉脚本、断言与 multipart，并可能写出 `null`。
  另外导入产出的 item 同样要满足「文件夹无 `request`、请求无 `items`」，否则侧栏只显示一行。
- **持锁期间不许调用会再次加锁的方法**。`mount_collection` 里写成
  `state.collections.lock().unwrap().get(uid).cloned().or_else(|| state.mount(..))` 时，
  `mount()` 内部会再锁 `collections` → 自死锁，表现是「点集合永远转圈、应用半死」。
  正确写法是先 `let cached = { let g = state.collections.lock().unwrap(); g.get(&uid).cloned() };`，
  离开作用域后再决定是否 `mount`。
- **`main:workspace-config-updated` 的 payload 是三元组** `(workspacePath, workspaceUid, workspaceConfig)`，
  且 uid/path 必须是**当前激活工作区**的。前端 `workspaceConfigUpdatedEvent` 会用自己 Redux 里的
  `activeWorkspace.uid` 做比对，不一致就直接丢弃（表现是「改了 remote/集合列表但侧栏没反应」）。
  统一入口：`commands::emit_workspace_config(app, state)`，不要再手拼。
- **`main:workspace-opened` 里只有 `workspaceConfig.type === 'default'` 才会触发 `switchWorkspace`**。
  启动恢复时要把「当前激活的那条」的 `type` 强制成 `default`，否则前端只更新列表不切过去。
  另外 `startupWorkspaceRestorePending` 由 `main:workspaces-ready` 清除，顺序不能颠倒。
- **非默认工作区不自动收录打开的集合**。`state.remember()` 只对默认工作区生效；否则「切到工作区 B 后
  打开集合 A 的文件」会把 A 静默写进 B 的 `workspace.yml`。显式收录只走 `add_collection_to_workspace`。
- **`MenuDropdown` 的菜单项 DOM 是 `data-testid="<父 testId>-<item.id>"`**（`role="menuitem"` 只在开启
  键盘导航后才出现）。新增菜单项必须给 `id` 才能被自动化定位；脚本里打开菜单要
  连发 `pointerdown/mousedown/pointerup/mouseup/click`（tippy 只认 pointer 系列事件）。
- **WS / gRPC 的 `payload.request` 是整个 item**（可能带 `draft`），真正的请求细节在
  `item.draft.request` / `item.request`，连接 id 用 `item.uid`。直接把它当 request 读 `request.url`
  会永远拿到空值（前端 `startWsConnection` 传的就是 `item.draft || item`）。
- **`renderer:ws:send-message` 的第三个参数是整条消息对象**（`{name, content, type, selected}`），
  不是裸字符串。直接 `serde_json::to_string` 会把 JSON 外壳一起发出去（对端收到的是
  `{"name":...,"content":...}`）；必须取 `.content`，并按 `.type` 决定文本还是二进制帧。
  同理 `renderer:ws:queue-message` 的 `selectedMessageIndex` 从 0 开始，指向 `body.ws[i]`。
- **`connections-changed` 的 payload 必须是 `[ids]`**（只含一个元素的数组）：shim 的 `on` 桥接会
  `handler(...payload)` 展开数组，直接发 `ids` 会变成多个实参。
- **主动断开也要发 `main:ws:close`**：前端靠 close 事件把状态条从 DISCONNECTING 收尾；
  服务端先关或读流异常结束时补一条 `code: 1006`，不要让 UI 停在中间态。
- **同一 requestId 重连时 hub 先关旧连接**，并用自增代号判断 `on_exit` 是否属于当前连接：
  否则旧任务的 `on_exit` 会把新连接从注册表里摘掉（表现：连上了但 `send-message` 一直 false）。
- **WS / gRPC 落盘必须写 `websocket:` / `grpc:` 段**，`info.type` 分别是 `websocket` / `grpc`；
  写进 `http:` 或只改 `info.type`，重新打开会退化成 HTTP 请求（前端按 `info.type` 选面板）。
- **集合 Runner 只跑 `http-request` / `graphql-request`**：WS / gRPC 是长连接，批量跑会挂住。

### 已知范围（尚未覆盖）

- **脚本**：无 `require` / `pm.*` / `Buffer` / 异步 `await`；不支持 `bru.runRequest`、`bru.sendRequest`。
  脚本错误会作为一条失败 test 显示，不中断请求。
- **认证**：NTLM、OAuth1、Akamai EdgeGrid 仍是占位；OAuth2 的 authorization_code / implicit
  需要浏览器回调，未接；AWSv4 的 multipart 请求体使用 `UNSIGNED-PAYLOAD`。
- **请求体**：GraphQL body 未处理；file 请求体只发送选中的第一条。
- **导入**：只支持 Postman 集合（`.json`）与 Bruno / OpenCollection 集合 zip；Postman 环境文件
  （`.postman_environment.json`）、Insomnia / OpenAPI / WSDL 导入未接；`preserveScripts` 选项被忽略
  （脚本始终保留并做 `pm.*` 翻译）。
- **Cookie**：`SameSite` 只存不发；不带 `Path` 的 cookie 默认按 `/` 处理；不做 public-suffix 校验。
- **文件监听**：轮询（0.7s）而非 inotify，超大集合（上万文件）会偏耗 CPU；工作区目录（`workspace.yml`）未监听。
- **WebSocket / gRPC**：连接、收发、断开、方法枚举、示例消息都已接上，但只有 HTTP 才跑脚本与断言
  （`grpc:script-*` / `grpc:test-results` 未编排）；两者都不走代理，TLS 只信任 webpki 内置根证书
  （自签 / 自定义 CA / 跳过校验都不支持，`wss` 与 `https` 内网自签服务会握手失败）。
  WS 消息类型沿用前端的 `text` / `json` / `xml`，没有二进制编辑器：收到的二进制帧按 UTF-8 宽松解码显示，
  另给 `messageHexdump`（与原版 Electron 行为一致）。
- 长尾子系统：Mock Server、终端；API Spec / WSDL / GraphQL schema 拉取；
  瞬态请求（transient request）、跨格式移动、`file-index` 数据层。这些前端有入口但后端通道未接。
- **明确不做**：OpenAPI 同步（`renderer:openapi-sync-*`）与 AI 相关（`renderer:ai-*`、AI 聊天面板）。
- **工作区**：创建 / 打开 / 改名 / 关闭 / 文档 / 环境导入可用，但「导出工作区 zip」只导配置不导集合内容；
  集合跨工作区共享时，移除只影响当前工作区（这正是预期，但 UI 上没有提示）
- **Git**：只有「克隆仓库」与侧栏的 connect / update / disconnect；不做 diff / commit / push / pull。
- **快照**：标签页 / 选中环境 / 侧栏宽度可恢复；窗口位置与 DevTools 内页签位置未持久化
- 启动仍采用「上次激活的工作区 + 其集合按快照恢复标签页」，未做「恢复上次滚动位置」
- CLI（`bru run`）后置

### IPC 契约（shim 映射）

| Bruno 通道 | Tauri command |
|---|---|
| `renderer:ready` / `renderer:app-ready` | `renderer_ready` / `app_ready` |
| `renderer:get-collection-security-config` | `get_collection_security_config` |
| `renderer:browse-directory` | `browse_directory` |
| `renderer:open-collection` / `renderer:mount-collection` | `open_collection` / `mount_collection` |
| `renderer:open-multiple-collections` | `open_multiple_collections` |
| `renderer:load-workspace-collections` | `load_workspace_collections` |
| `renderer:add-collection-to-workspace` | `add_collection_to_workspace` |
| `renderer:mount-workspace-scratch` | `mount_workspace_scratch` |
| `renderer:load-request` / `renderer:save-request` | `load_request` / `save_request` |
| `renderer:fetch-oauth2-credentials` / `refresh-oauth2-credentials` | `fetch_oauth2_credentials` / `refresh_oauth2_credentials` |
| `renderer:convert-postman-to-bruno` | `convert_postman_to_bruno` |
| `renderer:import-collection` | `import_collection` |
| `send-http-request` / `cancel-http-request` | `send_http_request` / `cancel_http_request` |
| `renderer:new-request` / `new-folder` / `delete-item` / `move-item` | `new_request` / `new_folder` / `delete_item` / `move_item` |
| `renderer:rename-item-name` / `rename-item-filename` / `resequence-items` | 同名 command |
| `renderer:create-collection` / `clone-collection` / `clone-folder` / `rename-collection` | 同名 command |
| `renderer:remove-collection` / `remove-collection-from-workspace` | 同名 command |
| `renderer:save-collection-root` / `save-folder-root` / `save-file` | 同名 command |
| `renderer:create-environment` / `save-environment` / `delete-environment` | 同名 command |
| `renderer:rename-environment` / `update-environment-color` / `save-environment-extends` | 同名 command |
| 工作区 / 全局环境通道 | `commands/env.rs` 内同名 command |
| `renderer:add-cookie` / `modify-cookie` / `delete-cookie` / `delete-cookies-for-domain` | `commands/cookie.rs` |
| `renderer:get-parsed-cookie` / `create-cookie-string` | `get_parsed_cookie` / `create_cookie_string` |
| `renderer:get-preferences` / `save-preferences` | 同名 command |
| `renderer:snapshot:*` | `snapshot_get` / `snapshot_save` / `snapshot_clear` / `snapshot_get_tabs` |
| `renderer:snapshot:get-collection` / `-get-sidebar` | `snapshot_get_collection` / `snapshot_get_sidebar` |
| `renderer:update-ui-state-snapshot` | `snapshot_update_ui_state` |
| `renderer:theme-change` | `theme_change` |
| `renderer:get-system-proxy-variables` / `refresh-system-proxy` | 同名 command |
| `renderer:refresh-pac-cache` / `clear-http-https-agent-cache` | 同名 command |
| `renderer:run-collection-folder` | `run_collection_folder` |
| `renderer:export-collection-zip` / `-yaml` / `-postman` / `export-environment` / `export-workspace` | 同名 command |
| `renderer:clone-git-repository` / `list-remote-branches-for-url` / `get-collection-git-remote-url` | 同名 command |
| `renderer:connect-collection-to-git` / `disconnect-collection-from-git` | 同名 command |
| `renderer:add-collection-watcher` / `start-workspace-watcher` | `add_collection_watcher` / `start_workspace_watcher` |
| `renderer:exists-sync` / `resolve-path` | `exists_sync` / `resolve_path` |
| `renderer:update-bruno-config` | `update_bruno_config` |
| `renderer:get-collection-workspaces` / `set-collection-workspace` | 同名 command |
| `renderer:ensure-collections-folder` / `find-unique-folder-name` | 同名 command |
| `renderer:is-bruno-collection-zip` / `import-collection-zip` | 同名 command |
| `renderer:create-workspace` / `open-workspace` / `open-workspace-dialog` | 同名 command |
| `renderer:rename-workspace` / `close-workspace` / `save-workspace-docs` | 同名 command |
| `renderer:import-workspace` / `import-workspace-environment` | 同名 command |
| `renderer:get-last-opened-workspaces` | `load_last_opened_workspaces` |
| `renderer:save-collection-security-config` | `save_collection_security_config` |
| `renderer:load-large-request` / `-via-worker` | 都转 `load_request` |
| `renderer:ws:start-connection` / `queue-message` / `send-message` | `commands/ws.rs` 同名 command（`ws_start_connection` 等） |
| `renderer:ws:close-connection` / `connection-status` / `is-connection-active` | `ws_close_connection` / `ws_connection_status` / `ws_is_connection_active` |
| `grpc:start-connection` / `send-message` / `cancel` / `end` | `commands/grpc.rs` 同名 command |
| `grpc:cancel-request` / `end-request` / `is-connection-active` | `grpc_cancel_request` / `grpc_end_request` / `grpc_is_connection_active` |
| `grpc:load-methods-proto` / `load-methods-reflection` | 同名 command |
| `grpc:generate-sample-message` / `generate-grpcurl` | 同名 command |

主→渲染事件名完全一致：`main:collection-opened`、`main:collection-tree-loaded`、`main:load-preferences`、
`main:workspace-opened`、`main:workspace-config-updated`、`main:app-loaded`、`main:hydrate-app-with-ui-state-snapshot`。

WS / gRPC 的事件同样沿用原版名字。payload 统一是三元组数组 `[requestId, collectionUid, eventData]`
（shim 会展开成 `handler(requestId, collectionUid, eventData)`），`requestId` 就是 `item.uid`：

- WS 实发：`main:ws:connecting`、`main:ws:open`、`main:ws:upgrade`、`main:ws:request`、`main:ws:message`、
  `main:ws:error`、`main:ws:disconnecting`、`main:ws:close`、`main:ws:connections-changed`。
  前端还监听 `main:ws:redirect`（原版用于 HTTP 升级跳转），本实现不发。
- gRPC 实发：`grpc:request`、`grpc:message`、`grpc:response`、`grpc:metadata`、`grpc:status`、`grpc:error`、
  `grpc:server-end-stream`、`grpc:connections-changed`。
  前端还会监听 `grpc:script-error`、`grpc:test-results`、`grpc:server-cancel-stream`——因为 gRPC 脚本编排未接，
  这三个目前不会发出（不影响面板可用）。注意 `grpc:before-call-start` 这类名字是**磁盘上的脚本钩子键**，
  不是事件。
- `main:ws:connections-changed` 与 `grpc:connections-changed` 的 payload 是**只含一个元素的数组**
  `[[id, ...]]`，前端 `updateActiveConnectionsInStore` 直接把它当活动连接 id 列表用，包错层级侧栏状态不刷新。

## License

MIT
