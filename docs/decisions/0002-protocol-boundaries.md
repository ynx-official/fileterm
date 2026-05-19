# ADR-0002: SSH/SFTP 与 FTP 保持协议边界

## 状态

Accepted

## 背景

SSH/SFTP 和 FTP 都涉及远程文件，但它们的会话模型不同。SSH/SFTP 共享认证上下文和目标主机，终端与文件面板天然联动；FTP 没有 shell，文件操作是完整主路径。

## 决策

SSH/SFTP 与 FTP 在 controller/protocol 层保持分离：

- SSH 会话负责 shell 与 SFTP 能力。
- FTP 会话只负责 FTP 文件能力。
- UI 可以共享文件面板体验，但不把协议能力揉成一个含大量空能力的 `RemoteSession`。

## 影响

- 类型更准确，布局逻辑更清楚。
- 后续支持端口转发、远端命令、FTPS 等能力时，不会污染另一类协议。
- 公共传输体验应通过 transfer system 收敛，而不是通过伪统一协议模型实现。
