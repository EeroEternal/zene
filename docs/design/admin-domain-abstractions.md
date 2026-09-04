# Admin 领域抽象优化方案

## 目的

Admin UI 最近的修复集中暴露出一组重复问题：API 响应类型和实体类型重复声明，Project 统计 SQL 重复，Billing 与 Project 使用的统计口径容易漂移，权限判断散落在各个 handler，页面通过不同的 query 参数互相跳转，i18n fallback 与 locale 文案也可能不一致。

这些问题不应继续通过页面级补丁解决。本方案定义一套渐进式抽象，用于降低契约漂移和重复实现，同时保持当前 Admin 的页面结构与接口行为稳定。

本文是开发设计稿，归档位置为 `docs/dev/`。已经落地的结论应同步到 `docs/architecture.md` 或 `docs/changelog.md`，并删除不再有效的计划内容。

## 设计原则

1. **按变化原因划分边界**：API 契约、资源请求、React 状态、数据库查询、权限策略和导航协议分别抽象。
2. **先统一契约，再重构实现**：先建立共享类型和查询口径，再迁移页面与 handler，避免抽象层本身产生新的漂移。
3. **领域优先于技术通用化**：Project、Organization、API Key 和 Billing 可以共享基础工具，但不合并成万能 CRUD 或泛化租户实体。
4. **读模型与数据库实体分离**：列表统计、详情信息和下拉选项按用途定义 DTO，不直接把数据库模型当作全部 API 契约。
5. **不改变业务行为的优先**：第一阶段只归拢类型和请求入口；权限、统计和安全语义的变化必须有测试和明确说明。
6. **安全边界显式化**：完整 API Key 只能出现在创建或轮换结果中，列表与详情只能使用前缀或脱敏值。

## 当前问题与目标边界

### 前端契约重复

`ApiResponse<T>`、Organization、Project、ApiKey 目前在多个页面和组件中重复声明。相同后端实体的字段集合不一致，容易出现某个页面更新了类型而其他页面仍使用旧契约。

目标是按领域集中类型：

- `admin/src/lib/api-types/common.ts`：共享 API 响应和分页类型。
- `admin/src/lib/api-types/tenancy.ts`：Organization、ProjectSummary、ProjectOption。
- `admin/src/lib/api-types/api-keys.ts`：ApiKeyListItem、ApiKeyDetail、ApiKeySecretResult。
- `admin/src/lib/api-types/billing.ts`：BillingQuery、BillingOverview、BillingTrendPoint。

旧模块可以在迁移期间保留 re-export，但不得继续定义新的重复类型。

### 资源请求与页面状态耦合

页面目前直接拼接 API URL、查询参数并解析响应。目标是为高频领域建立有名字的 resource 函数：

- `listProjects`、`getProject`、`createProject`、`deleteProject`。
- `listOrganizations`、`createOrganization`。
- `listApiKeys`、`rotateApiKey`、`updateApiKey`。
- `getBillingOverview`、`getBillingTrend`。

Resource 函数负责路径、参数编码和响应类型；Hook 负责加载状态、刷新和页面生命周期；Page 负责布局、交互编排和局部 UI 状态。

### Project 统计查询重复

`src/db/operations/projects.rs` 中列表和详情查询重复 API Key 数量、Token 用量和最后使用时间的聚合逻辑。目标是让列表与详情共享同一套统计口径和查询组成，避免增加字段时漏改。

当前统计口径必须明确记录为：

- 只统计 `status = 'success'` 的请求。
- 排除 `request_type = 'health_check'` 的请求。
- Token 用量使用 `request_logs.tokens_used`。
- 时间字段使用请求日志的 `created_at`。

Billing overview、Billing trend 和 Project summary 如果使用不同口径，必须在 DTO 和文档中明确说明，而不是隐式分叉。

### 权限判断散落

Admin handler 中存在平台管理员、组织管理员、资源归属者和实例授权等多种判断。目标是在 `src/admin/auth/authorize.rs` 中集中资源访问和变更策略，并让 helper 在成功时返回已加载资源，避免重复查询。

推荐的策略边界：

- `require_admin`：平台管理员权限。
- `require_org_admin`：平台管理员或当前组织管理员。
- `require_project_access`：检查项目归属并返回 Project。
- `require_api_key_access`：检查组织、owner 或平台权限并返回 ApiKey。
- `require_provider_access`：保留现有实例访问策略并作为统一入口。

每个 helper 只表达一个权限事实，不负责构造 HTTP 响应；handler 统一把 `AuthzError` 转成 API 响应。

### 导航参数语义不统一

当前页面使用 `org_id`、`select`、`edit`、`api_key_id`、`create` 等参数。它们不应强行合并为一个参数，但必须明确语义和消费方式：

| 参数 | 语义 |
| --- | --- |
| `select` | 选择或打开一个实体 |
| `edit` | 打开一个实体的编辑状态 |
| `create=1` | 打开创建流程 |
| `org_id` | 组织过滤范围 |
| `project_id` | 项目过滤范围 |
| `api_key_id` | 日志或计费过滤范围 |

`admin/src/lib/navigation.ts` 只负责生成高频、可复用的目标 URL；页面仍负责读取参数、校验资源是否存在以及在消费后使用 `replace` 清理一次性参数。

### i18n key 语义重复

同一 API Key 安全提示目前有多个近义 key。目标是按场景命名，而不是按组件命名：

- `apiKeys.fullKey.createdOnce`：创建成功后的完整密钥提示。
- `apiKeys.fullKey.rotatedOnce`：轮换成功后的完整密钥提示。
- `apiKeys.fullKey.unavailable`：详情页无法恢复完整密钥的提示。
- `apiKeys.fullKey.copy`、`apiKeys.fullKey.copied`：复制操作反馈。

高风险模块迁移后，组件调用 `t('namespace.key')`，不再依赖与 locale 可能分叉的英文或中文 fallback。所有用户可见 key 必须同时存在于 `zh.ts` 和 `en.ts`。

## 目标依赖关系

```text
Page
 └── Domain Hook
      └── Resource API
           └── apiCall
                └── Admin Handler
                     ├── Authorization Policy
                     └── Database Operation
```

各层职责如下：

| 层 | 负责 | 不负责 |
| --- | --- | --- |
| Page | 布局、交互和局部显示状态 | 拼接领域 API、定义领域实体类型 |
| Domain Hook | 加载、刷新、选择和提交状态 | 定义数据库统计口径 |
| Resource API | URL、query、body 和响应类型 | React 生命周期和页面布局 |
| Admin Handler | HTTP 输入输出、DTO 转换和错误映射 | 在循环中实现复杂聚合查询 |
| Authorization Policy | 资源访问和变更权限 | 直接写 HTTP 响应 |
| Database Operation | 查询、聚合和持久化 | 翻译用户文案 |

## 分阶段落地计划

### 阶段一：统一前端类型

1. 新建 `admin/src/lib/api-types/`。
2. 把 `ApiResponse<T>` 迁移到 common 类型入口。
3. 集中 Organization、Project、ApiKey 和 Billing DTO。
4. 旧路径仅保留兼容 re-export。
5. 删除迁移范围内页面和组件的重复声明。

验收标准：

- 迁移范围内不再出现重复的 `ApiResponse`、Organization、Project 定义。
- `ApiKey` 列表类型不再把名为 `key_hash` 的字段当作完整密钥使用。
- `pnpm typecheck`、前端测试和构建通过。

### 阶段二：抽取 Resource API

1. 建立 Projects、Organizations、API Keys 和 Billing resource 文件。
2. 页面改为调用有业务含义的函数，例如 `rotateApiKey(id)` 和 `getBillingTrend(query)`。
3. 保留现有 endpoint 和响应格式，不在该阶段同时改变后端行为。
4. 让 `useAdminCatalog` 复用 resource 函数，避免共享目录和页面各自请求同一资源。

验收标准：

- 领域 endpoint 和 query 参数只在 resource 层维护。
- 页面不再重复定义同一资源的响应类型。
- 错误映射仍由 `apiCall` 和领域 resource 保持一致。

### 阶段三：统一后端读模型和统计查询

1. 区分数据库实体、ProjectSummary、ProjectOption 和 ProjectDetail。
2. 抽取 Project summary 的共同 SELECT/JOIN 组成，或使用数据库 View。
3. 为 Billing overview、trend 和 breakdown 建立共享 `BillingQuery`。
4. 统一成功请求、健康检查、Token 和时间边界的统计规则。
5. 使用批量查询或 JOIN 解决 API Key 列表中的关联数据 N+1 问题。

验收标准：

- Project 列表和详情的统计字段来自同一查询口径。
- Billing trend 支持 day/hour、空数据和 scope filters。
- 趋势聚合结果与 overview 在相同过滤条件下可对账。
- 不因引入读模型而暴露完整 API Key。

### 阶段四：集中权限策略

1. 扩展 `src/admin/auth/authorize.rs` 的资源级 helper。
2. 迁移 Project、API Key、Provider 等 mutating handler。
3. 为查看、编辑、删除、轮换和配额操作建立权限矩阵测试。
4. 统一 forbidden、not found 和 database error 的映射语义。

验收标准：

- 新增资源变更 handler 不直接复制 `ctx.is_admin` 和 owner 判断。
- 非法跨组织访问不会因为 endpoint 不同而得到不同权限结果。
- 现有管理员、组织管理员和普通成员行为保持兼容，除非变更说明明确列出。

### 阶段五：导航和 i18n 收敛

1. 建立高频导航函数并统一 query 参数编码。
2. 明确一次性参数的消费和清理规则。
3. 合并 API Key 安全提示 key。
4. 清理 API Key、Project、Organization、Billing 和 Logs 中的旧 fallback。
5. 增加 locale key 对称性检查，确保中英文 key 集合一致。

## 非目标

本方案不包括以下工作：

- 不建立覆盖所有页面的通用 CRUD 框架。
- 不把 Organization、Project、Route 和 API Key 合并成泛化实体。
- 不建立使用字符串资源名的动态 API client，以免丢失 TypeScript 类型和领域语义。
- 不在一次变更中重写全部 Admin 页面。
- 不为了复用而合并不同领域的详情组件和权限规则。
- 不在没有统计对账和测试的情况下改变 Billing 或 Project 的业务口径。

## 当前实施进度

已完成第一批低风险抽象：

- 新增 `admin/src/lib/api-types/`，集中 common、tenancy、API Key、User 和 Billing 类型。
- 新增 Projects、Organizations、API Keys、Users、Billing、Providers、Routes 和 System resource API；API Key resource 同时覆盖模型配额和配额申请流程。
- 迁移 Projects、Organizations、Billing、Users 页面、Billing 筛选栏、Users MultiSelect、API Key 配额组件、API Key page hook、Provider/Route hooks、Routing Strategies 页面和共享 catalog 的主要请求入口。
- 保留旧 `components/services/types.ts` 的兼容类型导出，避免一次性扩大迁移范围；该文件不再定义 ApiKey 或 ApiResponse。
- Projects、Organizations、API Keys、Routes、Providers 和 Users 列表统一复用 `EntityListToolbar`；工具栏支持搜索、结果计数和可选筛选 actions，搜索字段在中英文 locale 中明确声明。
- Quota、Dashboard、Queue Lab、Smart Routing、Pool Settings、User Menu、Provider Insights 和 Route hooks 复用 common `ApiResponse`；Billing resource 增加可泛型化的明细分页查询。
- 新增 `admin-domain-resource` skill，并固化到 `.agents/skills/`，跨 gateway 产品复用的 types → resources → hooks → pages 流程。
- Provider、Provider Type、Route 和 System 类型分别集中到 `api-types/providers.ts`、`routes.ts`、`system.ts`，旧的 instances/services 类型文件仅保留兼容导出。
- Provider Type CRUD、Model Discovery、Provider Draft Test、Provider Capability Profile/Stats，以及 API Key quota/model-limit/project-quota 请求均已下沉到对应 Resource；页面和对话框不再直接拼接这些 endpoint。
- Conversation 生命周期请求、Dashboard 日志概览/图表请求和 Logs Hook 的分页与详情请求已分别下沉到 `resources/conversations.ts` 与 `resources/logs.ts`；Chat 的流式 completion 请求保留在页面以维护 AbortController/SSE 生命周期。
- Queue Lab、Request Analysis、Dashboard 的 pool metrics/queue 分析请求已下沉到 `resources/queue.ts`；Queue Lab 的轮询、表单校验和运行状态仍保留在页面。
- Analytics 的性能、top models/routes、billing breakdown 和 token breakdown 请求已下沉到 `resources/analytics.ts`，保留各可选报表请求的失败降级行为。
- Mock Services 的状态、启动和停止请求已下沉到 `resources/mock-services.ts`；页面继续负责轮询和动作反馈。
- Filter rules/logs/content、Scoped Quota、User change-password、Smart Router、Pool Settings、Provider Insights 和 Route binding cleanup 请求已分别下沉到 Filter、Quota、User、System、Provider 和 Route resources。
- 页面和组件中剩余的直接 API 请求仅包括认证/组织切换、WS ticket 和 Chat completion 等连接生命周期请求。
- 价目台账（`price-book`）按领域补齐 `api-types` / `resources` / `usePriceBookPage` / 页面，平台管理员可独立编辑进货与出货扁平价。
- 新增 `admin/src/lib/navigation.ts`，统一日志 API Key、Provider/Model/Chat、Route、API Key 和 Organization 的高频跳转 URL 及 query 编码；页面仍负责消费和清理一次性参数。
- 明确区分 `routeId`（API Key 创建时预选路由）与 `route_id`（日志/资源过滤参数），并在 Route 创建成功流程提供带路由和项目上下文的 API Key 创建入口。
- 未改变后端 endpoint、权限规则或统计口径。
- 已通过 `cd admin && npm run lint`、`cd admin && npm run build` 和 `cd admin && npm test -- --run`；新增 Billing overview/trend、API Key quota/model-limit、Provider Type/Model Discovery、Route、Conversation、Logs、Queue、Analytics、Mock Services、Filter、Quota、System、Provider Insights 与 User resource 查询契约测试。
- `cargo check --tests` 已通过；Admin authorization 集成测试仍需 PostgreSQL，当前本地执行因 `PoolTimedOut` 无法连接数据库。

仍可继续推进的非本轮目标：

- 评估是否需要稳定的 Domain Hook 模板。
- 收敛导航参数与高风险 i18n fallback。
- 在 PostgreSQL 集成环境中执行权限矩阵和 Billing 对账集成测试。

## 变更审查清单

提交涉及 Admin 领域抽象的变更时，应检查：

- 是否新增了重复的实体或 API 响应类型？
- 是否把页面业务逻辑错误地放进通用 UI 组件？
- 是否改变了统计口径、时区或过滤边界？
- 是否暴露了完整 API Key 或把 `key_hash` 当作 secret？
- 是否绕过了 `authorize.rs` 中已有的权限 helper？
- 是否同时更新了 `zh.ts` 和 `en.ts`？
- 是否补充了对应的前端测试、Rust 单元测试或集成测试？
- 是否只修改了本阶段目标，避免夹带整库重构？

## 相关文档和代码入口

- [整体架构](../architecture.md)
- [组织与用户模型](../architecture/org-user-model.md)
- [Admin 交互与详情体验计划](../dev/admin-ux-plan.md)
- [文档规范](../rules/documentation.md)
- `admin/src/components/services/types.ts`
- `admin/src/hooks/useAdminCatalog.ts`
- `admin/src/lib/billing-filters.ts`
- `src/admin/auth/authorize.rs`
- `src/db/operations/projects.rs`
