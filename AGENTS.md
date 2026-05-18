# TermDock Agent Guide

## 1. 项目定位

TermDock 是一个面向开发者与运维场景的桌面远程工作台，当前以 Electron + React + TypeScript 为主，目标是围绕 `SSH / SFTP / FTP` 做一个真正可日常使用的多标签桌面客户端。

当前仓库已经不是单纯脚手架，已经进入“骨架 + 核心链路打通中”的阶段。

当前一轮协作的视觉重点，是把主题系统继续搭稳：先统一设计 token、明暗主题变量、终端配色和组件皮肤，再让页面整体风格保持一致。

## 2. 当前仓库结构

```txt
termdock/
  AGENTS.md
  README.md
  package.json
  tsconfig.base.json
  docs/
    architecture.md
    roadmap.md
  .agents/
    README.md
    extensions/
      README.md
  apps/
    desktop/
      index.html
      package.json
      tsconfig.json
      tsconfig.node.json
      vite.config.ts
      src/
        main/
          main.ts
          ipc.ts
          services/
            file-profile-repository.ts
            local-files-service.ts
            session-controllers.ts
            workspace-service.ts
        preload/
          preload.cts
        renderer/
          App.tsx
          i18n.ts
          main.tsx
          vite-env.d.ts
          components/
            TerminalView.tsx
          styles/
            app.css
            themes/
              default.css
              index.css
              tokens.css
  packages/
    core/
      src/
        index.ts
    shared/
      src/
        index.ts
    storage/
      src/
        index.ts
```

## 3. 每层职责

- `apps/desktop`
  - 主桌面应用。
  - 包含 Electron `main`、`preload`、React `renderer` 三层。
- `apps/desktop/src/main`
  - 桌面进程入口、窗口创建、IPC 注册、系统能力接入。
- `apps/desktop/src/main/services`
  - 当前最核心的业务层。
  - 已承载 profile 存储、本地文件访问、工作区状态管理、SSH/FTP 会话控制。
- `apps/desktop/src/preload`
  - 向 renderer 暴露安全 API。
- `apps/desktop/src/renderer`
  - UI 工作台、连接管理窗口、文件面板、终端视图、样式主题。
- `packages/core`
  - 领域模型与共享业务类型。
  - 例如 `ConnectionProfile`、`WorkspaceSnapshot`、`SessionSnapshot`、`TransferTask`。
- `packages/storage`
  - 存储层抽象。
  - 当前提供 `ProfileRepository` 接口和内存仓储实现。
- `packages/shared`
  - 跨包共享常量与轻量工具。
  - 目前内容较少，后续适合承接 `constants / utils / zod schemas`。
- `docs`
  - 产品架构和路线图文档。
- `.agents`
  - 给人和 AI 协作者共用的扩展设计区，不放运行时代码。

## 4. 当前已经实现的功能

结合现有代码，当前已完成或已具备雏形的能力如下：

- Monorepo workspace 基础结构
- Electron 主窗口启动
- 独立连接管理器窗口
- React 工作台主界面
- 标签页工作区模型
- SSH / FTP 连接配置的新增、编辑、删除、持久化
- 基于文件的 profile 存储（`profiles.json`）
- 本地目录浏览
- 本地文件读写
- SSH 会话连接
- SSH shell 输出接入
- 终端写入与 resize IPC
- SFTP 远程目录浏览
- 远程文件读取与写回
- FTP 会话连接与远程文件能力
- 上传下载任务队列与进度状态
- 工作区快照广播到多个窗口
- 预览态数据与桌面运行态双模式

当前主题系统已经开始成形，主要落点包括：

- `apps/desktop/src/renderer/styles/themes/tokens.css`
  - 基础视觉 token，供半径、阴影、间距等全局样式复用。
- `apps/desktop/src/renderer/styles/themes/default-dark.css`
- `apps/desktop/src/renderer/styles/themes/default-light.css`
  - 明暗主题变量与组件覆盖层。
- `apps/desktop/src/renderer/hooks/useThemeMode.ts`
  - 通过 `document.documentElement.dataset.theme` 切换主题。
- `apps/desktop/src/renderer/components/TerminalView.tsx`
  - 从 CSS 变量读取终端主题色，确保终端外观和全局主题联动。

## 5. 当前代码状态判断

当前最重的实现集中在：

- `apps/desktop/src/main/services/workspace-service.ts`
  - 工作区状态中心，已经承担了 profile、tab、session、transfer 的主要调度职责。
- `apps/desktop/src/main/services/session-controllers.ts`
  - SSH / FTP 的运行时协议接入。
- `apps/desktop/src/renderer/App.tsx`
  - 目前 UI 逻辑较重，承接了较多页面状态与交互。

这意味着当前项目可以继续开发，但下一步很适合做“分层再整理”，否则功能继续增长后，`workspace-service.ts` 和 `App.tsx` 会迅速变成过重文件。

主题相关的判断也同样适用：新增样式不要直接在页面里硬写，优先判断它是不是应该进入 token、主题变量、通用组件皮肤或终端主题层。

## 6. 推荐的扩展方式

### 6.1 新功能接入原则

- 新协议优先拆成独立能力层，不要直接塞进现有 SSH 逻辑。
- 新窗口能力先定义 IPC 边界，再做 renderer 交互。
- 新状态优先进入 `packages/core` 定义类型，再下沉到 `main/services` 和 `renderer`。
- 新文件传输能力统一挂到 transfer 体系，不要各处单独维护进度。
- 新视觉能力优先进入 `styles/themes/` 的 token 和主题层，不要把颜色、阴影、圆角散落在业务组件里。
- 如果是终端、标签页、按钮、表格、面板这类共用外观，先判断是否需要补主题变量，再改单个组件。

### 6.2 推荐扩展路径

1. 先在 `.agents/extensions/` 写功能草案
2. 明确影响层级
3. 补充 `packages/core` 类型定义
4. 新建或扩展 `main/services/*`
5. 经由 `ipc.ts` 和 `preload.cts` 暴露接口
6. 最后接到 `renderer` 页面或组件
7. 如果涉及视觉样式，先收敛 token 和主题变量，再做组件级样式补丁

### 6.3 适合独立拆包的能力

后续一旦开始实做，建议尽快从 `packages/` 中独立出以下包：

- `packages/protocol-ssh`
  - SSH shell、SFTP adapter
- `packages/protocol-ftp`
  - FTP adapter
- `packages/ui`
  - 可复用界面组件、工作台壳、文件表格、状态徽标
- `packages/session`
  - 会话状态机、连接生命周期、重连策略

## 7. 建议规范后的目录结构

下面这套结构更适合这个项目继续长大，且和你现在的代码方向一致：

```txt
termdock/
  apps/
    desktop/
      src/
        main/
          main.ts
          ipc/
            app.ts
            workspace.ts
            terminal.ts
            remote-files.ts
            transfers.ts
          services/
            workspace/
              workspace-service.ts
              tab-service.ts
              transfer-service.ts
            profiles/
              file-profile-repository.ts
            local-files/
              local-files-service.ts
            sessions/
              session-controllers.ts
              ssh-session-controller.ts
              ftp-session-controller.ts
        preload/
          preload.cts
        renderer/
          app/
            App.tsx
            routes.tsx
          components/
          features/
            connections/
            workspace/
            terminal/
            remote-files/
            local-files/
            transfers/
          hooks/
          styles/
  packages/
    core/
    shared/
    storage/
    protocol-ssh/
    protocol-ftp/
    ui/
```

## 8. 当前最值得先做的结构整理

如果按优先级来，建议先做这几步：

1. 把 `apps/desktop/src/main/ipc.ts` 拆成按领域分文件。
2. 把 `workspace-service.ts` 按 `tabs / sessions / transfers` 拆子模块。
3. 把 `renderer/App.tsx` 中连接管理、文件面板、传输面板、顶部标签拆成 feature 组件。
4. 把 SSH 与 FTP controller 从同一个文件里分离。
5. 把共享类型继续收敛到 `packages/core`，避免 renderer 和 main 自己长类型。
6. 把主题系统继续做成“token -> theme vars -> component skins -> terminal colors”这条清晰链路，避免样式散写。

## 9. `.agents` 目录约定

`.agents` 目录建议只放协作与设计资料，不直接放生产运行代码。

建议用途：

- `extensions/`
  - 新功能提案、扩展设计、协议接入方案。
- 后续可继续增加：
  - `plans/`：阶段性开发计划
  - `decisions/`：架构决策记录
  - `checklists/`：发布、回归、联调清单

## 10. 协作约束

- `packages/core` 是领域模型单一事实源。
- renderer 不直接接协议客户端。
- 所有系统能力通过 `main -> preload -> renderer` 暴露。
- SSH/SFTP 与 FTP 继续保持分离，不做伪统一。
- 优先小步拆分已有大文件，而不是一边加功能一边继续堆。
- 视觉层优先保持一致性，尤其是主题、终端、标签页和表格这些高频界面。

## 11. 一句话结论

这个仓库目前已经有了一个可继续推进的 MVP 雏形；接下来最重要的不是“再多加一点功能”，而是边做功能边把 `协议层 / 服务层 / UI 层 / 类型层 / 主题层` 的边界立住。
