# TermDock 通用组件技能指南

## 一、项目结构概述

TermDock 是一个基于 **Electron + React + TypeScript** 的桌面远程工作台应用，采用 Monorepo 结构。

```
termdock/
├── apps/           # 应用层（可执行应用）
│   └── desktop/    # 桌面应用主目录
├── packages/       # 共享包（可复用模块）
│   ├── core/       # 领域模型的唯一数据源
│   ├── shared/     # 共享常量和轻量工具函数
│   └── storage/    # 存储抽象层
└── docs/           # 文档目录
```

### 核心技术栈

| 层级 | 技术 | 说明 |
|------|------|------|
| 框架 | Electron 28+ | 桌面应用容器 |
| 前端 | React 18 + TypeScript | UI 框架 |
| 构建 | Vite | 构建工具 |
| 终端 | xterm.js | 终端组件 |

---

## 二、前端代码结构

前端代码位于 `apps/desktop/src/renderer/`，采用功能模块划分：

### 2.1 Renderer 层结构

```
renderer/
├── components/     # 通用组件
├── features/       # 业务功能模块
│   ├── connections/    # 连接管理
│   ├── files/          # 文件管理
│   ├── commands/       # 命令面板
│   ├── transfers/      # 文件传输
│   ├── layout/         # 布局组件
│   ├── system/         # 系统信息
│   ├── workspace/      # 工作区
│   └── common/         # 通用功能组件
├── hooks/          # 自定义 Hooks
├── styles/         # 样式文件
├── App.tsx         # 根组件
└── main.tsx        # 入口文件
```

### 2.2 架构边界

```
┌─────────────────────────────────────────────────────────────┐
│                     Renderer (React)                        │
│  ┌───────────────────┐  ┌─────────────────────────────────┐ │
│  │  UI Components    │  │         Features                │ │
│  │  (TerminalView)   │  │ (connections, files, commands)  │ │
│  └────────┬──────────┘  └──────────────┬──────────────────┘ │
│           │                             │                    │
└───────────┼─────────────────────────────┼────────────────────┘
            │                             │
            ▼                             ▼
┌─────────────────────────────────────────────────────────────┐
│                      Preload                                │
│         安全的 IPC 桥梁，暴露系统能力给前端                    │
└─────────────────────────────────────────────────────────────┘
            │
            ▼
┌─────────────────────────────────────────────────────────────┐
│                      Main Process                           │
│  ┌───────────────────┐  ┌─────────────────────────────────┐ │
│  │      IPC Handlers │  │       Services                  │ │
│  │  (ipc/*.ts)       │  │ (ssh, ftp, workspace, files)   │ │
│  └───────────────────┘  └─────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

---

## 三、通用组件清单

### 3.1 主组件目录 `renderer/components/`

| 组件 | 文件 | 功能描述 |
|------|------|----------|
| **TerminalView** | `TerminalView.tsx` | 终端视图核心组件，基于 xterm.js，支持 SSH 会话展示、复制粘贴、搜索查找、右键菜单 |

### 3.2 通用功能组件 `renderer/features/common/`

| 组件 | 文件 | 功能描述 |
|------|------|----------|
| **AppIcon** | `AppIcon.tsx` | 统一 SVG 图标组件，支持 11 种图标类型 |
| **ContextMenu** | `ContextMenu.tsx` | 右键上下文菜单组件 |
| **handleHorizontalWheelScroll** | `horizontal-scroll.ts` | 水平滚动处理工具函数 |

### 3.3 组件功能详解

#### TerminalView（终端组件）
- 基于 `@xterm/xterm` 构建
- 支持主题颜色定制（CSS 变量）
- 快捷键支持：复制(⌘C/Ctrl+Shift+C)、粘贴(⌘V/Ctrl+Shift+V)、查找(⌘F/Ctrl+F)
- 右键上下文菜单
- 自动适配容器尺寸

#### AppIcon（图标组件）
- 支持图标类型：`grid`、`menu`、`server`、`connections`、`folder`、`file`、`history`、`refresh`、`upload`、`download`、`flash`
- 可自定义尺寸（默认 14px）
- 颜色继承父元素 `currentColor`

#### ContextMenu（右键菜单）
- 点击外部区域自动关闭
- Esc 键关闭
- 位置自动调整（避免超出视口）
- 支持禁用状态和危险操作样式

---

## 四、移动端开发方案

当前项目是纯桌面应用，如需支持移动端，推荐以下方案：

### 4.1 方案对比

| 方案 | 优势 | 劣势 | 推荐度 |
|------|------|------|--------|
| **React Native** | 复用 React 组件逻辑，TypeScript 支持好，原生性能 | 需要重新实现 UI，学习成本 | ⭐⭐⭐⭐⭐ |
| **Capacitor** | 保持 Web 技术栈，复用 renderer 层代码 | 性能略差于原生 | ⭐⭐⭐⭐ |
| **Flutter** | 跨平台一致性好，性能优秀 | 需要学习 Dart，无法复用现有代码 | ⭐⭐⭐ |

### 4.2 React Native 实施路径

```
1. 架构拆分
   └─ 确保 packages/core/ 与 UI 完全解耦

2. 项目初始化
   └─ 在 apps/ 下新建 mobile/ 目录
   └─ 初始化 React Native 项目

3. 代码复用
   └─ 复用 packages/core/ 和 packages/shared/
   └─ 重新实现移动端 UI 组件

4. 原生能力
   └─ 实现移动端存储和权限逻辑
   └─ 处理 SSH 连接的移动端适配
```

### 4.3 关键注意事项

1. **SSH 连接**：移动端需要使用原生 SSH 库或 WebSocket 代理
2. **存储方案**：使用 React Native AsyncStorage 或 Realm
3. **权限管理**：处理文件系统权限、网络权限等
4. **交互适配**：移动端触控交互与桌面端差异较大，需要重新设计

---

## 五、开发规范

### 5.1 代码约定

- 使用 TypeScript，严格模式
- 组件命名采用 PascalCase
- 文件命名采用 kebab-case
- 函数命名采用 camelCase

### 5.2 状态管理

- 领域状态优先进入 `packages/core`
- UI 状态在组件内部管理
- 跨组件状态通过 Context 或自定义 Hooks 管理

### 5.3 样式规范

- 主题样式优先走 `token -> theme vars -> component skins`
- 避免在业务组件中硬编码颜色、阴影、圆角
- 使用 CSS 变量进行主题切换

---

## 六、核心入口文件

| 文件 | 路径 | 说明 |
|------|------|------|
| 主进程入口 | `apps/desktop/src/main/main.ts` | Electron 主进程启动 |
| 渲染进程入口 | `apps/desktop/src/renderer/main.tsx` | React 应用启动 |
| IPC 注册 | `apps/desktop/src/main/ipc/index.ts` | 进程间通信定义 |
| 预加载脚本 | `apps/desktop/src/preload/preload.cts` | 安全 API 暴露 |

---

## 七、扩展开发建议

如需添加新功能，推荐路径：

1. 在 `.agents/extensions/` 或 `docs/plans/active/` 编写功能草案
2. 明确影响层级：`core` → `main services` → `ipc` → `preload` → `renderer`
3. 补充或复用 `packages/core` 类型
4. 新建或扩展 `main/services/*`
5. 经由 `ipc.ts` 和 `preload.cts` 暴露能力
6. 最后接到 renderer 页面或组件
