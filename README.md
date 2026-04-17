# Pathio Backend

Rust + Axum backend for Pathio knowledge-map workspace.

## Requirements

- Rust toolchain (cargo)
- PostgreSQL

## Environment

Create `.env` in `backend/`:

```env
DATABASE_URL=postgres://postgres:123@localhost:5432/pathio_db
PORT=3000
```

Initialize schema (first time):

- Run SQL in `init.sql` against your database.
- For existing deployments, run the backfill migration:

```bash
psql "$DATABASE_URL" -f migrations/20260414_billing_backfill.sql
```

## Run

```bash
cargo run
```

Health check:

```bash
GET http://127.0.0.1:3000/api/health
```

## Plan Limits & Billing

Quota is now driven by `plan_entitlements` in `init.sql` (not hardcoded constants).

Default seeds:

- `free`: 3 roadmaps, 50 nodes per workspace, 2 members
- `team`: uncapped core quotas
- `enterprise`: uncapped + enterprise flags (SSO/audit/private deployment)

Behavior notes:

- Limits are enforced on `POST /api/roadmaps` and `POST /api/nodes`.
- When limit is hit, backend returns `402 Payment Required`.
- Existing nodes can still be edited, moved, renamed, status-updated, and deleted.
- After deleting nodes, free capacity is available again.

Billing APIs:

- `GET /api/billing/plans`
- `GET /api/billing/subscription`
- `POST /api/billing/checkout-session`
- `POST /api/billing/webhook`

Event API:

- `POST /api/events` (allowlist: roadmap/upgrade/checkout/invite/share related events)

## Concurrency Safety

`create_roadmap` and `create_node` run quota checks inside a DB transaction and lock the organization row (`FOR UPDATE`) before count + insert, preventing concurrent over-limit inserts.

## Validate Locally

Compile checks:

```bash
cargo check
cargo test --no-run
```

Recommended API checks:

- Free user can create roadmap #2 and #3, roadmap #4 returns `402`.
- Free workspace can create node #1..#50, node #51 returns `402`.
- Free workspace invitation: member #3 returns `402`.
- `POST /api/billing/checkout-session` creates a pending checkout session.
- `POST /api/billing/webhook` with `paid` updates org to paid plan and unblocks quotas.

## Migration Order & Rollback

Recommended order:

1. Backup database.
2. Run `init.sql` for new environments, or run `migrations/20260414_billing_backfill.sql` for existing environments.
3. Start backend and verify:
   - `GET /api/health` = 200
   - `GET /api/billing/plans` = 200
   - `GET /api/org/details` = 200 (after login token)
4. Use `docs/db-checklist.md` for table/column/member consistency checks.

Rollback notes:

- Current migration is additive and non-destructive.
- If rollback is needed, disable billing/event routes first, then drop added tables/columns manually after backup.
