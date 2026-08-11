# Zene Cloud Platform

Zene Cloud Platform 是一个面向团队和个人开发者的云端 Coding Agent 产品方案。它以 Zene 作为 Agent Runtime、Keel 作为沙箱内的执行策略层，提供账号、代码仓库、隔离工作区、长时间运行的 Agent、实时协作、代码阅读、Git 分支与 Pull Request 全流程。

当前目录先作为独立仓库的设计种子，不包含生产实现。

## 文档

- [产品与系统设计](docs/PRODUCT_AND_SYSTEM_DESIGN.md)

## 核心结论

- 云端控制面与 Agent 执行面必须分离。
- 每次 Agent Run 使用独立 microVM 或强隔离容器；Keel 是第二层约束，不能替代租户级隔离。
- Zene 通过 ACP 运行在 Worker 内，云端不直接暴露现有本地 `zene-gateway`。
- GitHub App 是首个 Git 集成，所有 clone、commit、push、PR 操作都使用短期凭证与可审计动作。
- UI 采用“左侧导航 + 中央 Agent 工作区 + 右侧代码/变更/终端检查器”的信息架构，交互可参考 Cursor Agent，但使用独立品牌与组件实现。
- Postgres 保存业务真相，Redis 保存短期协调状态，S3 兼容对象存储保存日志、补丁和快照，事件流支持断线续传。

## 建议仓库名

`zene-cloud`

## 建议许可证

在明确商业模式前保持私有仓库；若开源，控制面与 Worker 可分别选择许可证。
