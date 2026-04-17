# Pathio 商业评估报告（双市场）

- 版本：v1
- 日期：2026-04-13
- 约束：不改变现有 UI 与视觉风格
- 核心 ICP：小团队研发/内容团队（PLG 优先）

## 1. 结论摘要
- Pathio 当前最强差异化是“路径图 + 深度笔记 + 可分享只读视图”的一体化工作流。
- 现有 Free 配额（3 路图 / 50 节点 / 2 成员）适合 PLG 试用与触发升级。
- 主要闭环缺口已补齐：`plans`、`subscription`、`checkout-session`、`webhook`、事件埋点。

## 2. 竞品矩阵（双市场）
| 产品 | 目标客群 | 免费版策略 | 团队协作能力 | 企业能力 | 价格锚点 |
|---|---|---|---|---|---|
| Notion | 通用知识协作 | 有 Free，配额递进 | 强 | 强 | 官方页按席位月付 |
| Miro | 白板协作 | Free 可用，团队功能分层 | 强 | 强 | 官方页按席位月付 |
| Whimsical | 轻白板/流程图 | Free 限制项目与协作深度 | 中-强 | 中 | 官方页按席位月付 |
| Heptabase | 深度知识工作流 | 偏个人深度 | 中 | 弱-中 | 官方页订阅制 |
| Taskade | AI+协作任务 | Free 可用 | 中 | 中 | 官方页分层订阅 |
| ProcessOn | 中文图形协作 | 国内流量入口强 | 中 | 中 | 人民币订阅锚点 |
| Xmind | 思维导图 | 个人到团队扩展 | 中 | 中 | 中英文双价格体系 |

## 3. Pathio 定价与分层建议
- Free（体验层）：保留 3/50/2，强化分享传播，持续触发 402 升级。
- Team（主收入层）：
  - 中国：30 RMB/seat/月（已落地）
  - 全球：9 USD/seat/月（已在 entitlement 默认值中）
  - 核心权益：去配额上限、团队协作沉淀、更稳定组织工作流。
- Enterprise（询价层）：SSO、审计、私有化、专属支持（后端能力位已预留）。

## 4. 商业漏斗定义（AARRR）
- Acquisition：分享链接访问、注册来源、落地页转化。
- Activation：首图、首连线、首笔记、首分享。
- Revenue：`checkout_started` -> `checkout_succeeded`。
- Retention：7/30 日活跃组织、周笔记更新率、团队协作深度。
- Expansion：成员增长、Team -> Enterprise 线索。

## 5. KPI 看板（建议）
- 周级：注册数、激活率、402触发率、checkout started、paid conversion。
- 月级：MRR、ARPA、7/30日组织留存、团队席位净增长。

## 6. 风险与应对
- 风险：Webhook 对接真实支付前，当前为 mock gateway。
- 应对：下一阶段接 Stripe/支付宝/微信，保持现有 API 契约不变。
- 风险：前端文案与配额口径易漂移。
- 应对：以 `plan_entitlements` 为单一真相源，后台与文案定期对齐。

## 7. 参考基线
- https://www.notion.com/pricing
- https://miro.com/pricing/
- https://whimsical.com/pricing
- https://heptabase.com/pricing
- https://www.taskade.com/pricing
- https://www.processon.com/upgrade
- https://xmind.com/pricing
- https://xmind.app/cn/in-app/pricing/
