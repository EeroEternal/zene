---
name: admin-domain-resource
description: Build reusable Admin domain modules with typed API contracts, resource functions, domain hooks, and stable entity-list UI. Use when adding or migrating gateway Admin pages and when the same domain pattern should be reused across products.
---

# Admin domain resource

Use this skill when a gateway Admin feature needs to be reusable across products. The goal is a typed, domain-oriented vertical slice, not a universal CRUD framework.

## Required reading

- `AGENTS.md`
- `docs/design.md`
- `docs/ai/agents/ui-entry.md`
- `docs/design/admin-domain-abstractions.md`
- For visible list work: `docs/design/components.md` and `docs/design/layout.md`

## Layer contract

Use this dependency direction:

```text
Page -> domain hook -> resource API -> api client -> Admin handler
```

- `api-types/<domain>.ts`: DTOs and option/read-model contracts. Separate list, detail, option, and secret-result shapes.
- `resources/<domain>.ts`: named functions own paths, query encoding, request bodies, and `ApiResponse` types.
- Domain hooks: loading, refresh, selection, mutation state, and lifecycle behavior.
- Pages: layout, composition, and local UI state. Pages must not assemble domain URLs or duplicate DTOs.
- Shared components: interaction patterns only. Do not put domain authorization or business rules into generic components.

## Resource rules

- Prefer named functions such as `listProjects`, `listProjectOptions`, `rotateApiKey`, or `getBillingTrend`.
- Keep endpoint paths and query parameter names in one resource module.
- Provide a narrower option function when a selector does not need a full entity.
- Preserve endpoint behavior and statistics semantics during structural migration.
- Never expose a complete API secret in list/detail DTOs; only create/rotate results may contain it.
- Do not create a string-keyed dynamic client or a generic `useCrudResource` abstraction.
- Keep authorization in backend policy helpers; frontend resource functions do not replace authorization.

## Reusable UI rules

- Use `EntityListToolbar` for entity-list search and result counts.
- Search placeholders must enumerate fields actually searched; matching is normally case-insensitive substring matching.
- Keep organization and project labels separate. Select options show one entity identity only.
- Follow the canonical list and master-detail patterns in `docs/design/components.md`.
- All visible copy uses `t()` and both locale files are updated for new keys.
- Reuse semantic tokens and shared UI primitives; do not introduce page-specific button or selection dialects.

## Migration workflow

1. Inventory existing endpoints, DTOs, page state, and tests.
2. Define domain types, including narrow option/read-model types.
3. Implement resource functions and resource contract tests for query/body encoding.
4. Move page requests to resources; keep lifecycle state in the page or domain hook.
5. Extract only stable interaction patterns into shared components.
6. Verify no migrated page defines duplicate domain DTOs or assembles migrated URLs.
7. Run focused tests, then `cd admin && npm run lint`, `npm run build`, and `npm test -- --run`.
8. Record the migrated domain and remaining direct requests in `docs/design/admin-domain-abstractions.md`.

## Acceptance checklist

- [ ] API types are domain-owned and distinguish list/detail/options/secrets.
- [ ] Resource functions own URLs, query parameters, and response types.
- [ ] Page and component code does not duplicate migrated DTOs.
- [ ] Shared UI component contains no domain-specific authorization or mutation logic.
- [ ] Search placeholder matches the implemented fields.
- [ ] `zh.ts` and `en.ts` are symmetric for any new user-visible keys.
- [ ] Resource or page tests cover the migration's behavior.
- [ ] Lint, build, and Admin tests pass.
