---
name: verify-design-doc
description: Validate a design doc's claims about existing code, its SQL, and its diagrams by actually running them before commit. Use when writing or reviewing a docs/dev/ proposal that cites current code, proposes migrations/DDL, or embeds mermaid.
---

# Verify a design doc against real code and a real database

## Symptom / misjudgment

Two failures that read as correct on the page:

1. **Phantom capability.** The doc cites a builtin plugin, struct field, or dependency as existing. Reviewers schedule work on top of it. In reality `src/plugins/builtin/jwt_auth.rs` returns `Continue` from all three hooks, `PluginHookContext` has no identity field, and `Cargo.toml` has no JWT/OIDC/LDAP dependency at all. This is AGENTS.md rule 2 (禁止幻觉代码) in doc form.
2. **DDL that fails on execution.** The SQL parses and looks obviously right, yet breaks on real data. Both of these shipped in a reviewed doc and were only caught by running it:
   - `ADD COLUMN scope_kind ... DEFAULT 'org'` silently relabeled the two pre-existing **platform** roles as org roles, which a later rule would reject — platform admins would load with zero permissions.
   - `UNIQUE (user_id, role_id, scope_kind, scope_id)` does not dedupe when `scope_id IS NULL`; the same platform binding inserted twice without error.

## Root cause

Design review reads prose. Whether a trait is actually implemented, and how DDL defaults plus SQL NULL semantics behave against existing rows, are not reliably decidable by reading.

## Procedure

### 1. Prove each claim about current code

Cite a command output, not memory. A named symbol existing is not the same as it doing anything:

```bash
rg 'jsonwebtoken|openidconnect|ldap3' Cargo.toml   # claimed dependency
rg 'org_id|project_id|api_key_id' src/plugins/builtin/   # claimed consumer of a field
rg -n 'Extension\(_' src/admin/                    # ctx taken then discarded == no authz
```

For a plugin or trait impl claimed to be functional, compare its `Continue` count against `Mutated`/`Reject`; all-`Continue` means skeleton.

### 2. Run every SQL block on a throwaway Postgres

Apply the repo's real migrations first, then the doc's SQL on top — never a hand-written approximation of the schema:

```bash
export PATH=/usr/lib/postgresql/16/bin:$PATH
initdb -D /tmp/pgdata -U postgres --auth=trust
pg_ctl -D /tmp/pgdata -o "-k /tmp/pgrun -p 55432 -c listen_addresses=''" -l /tmp/pg.log start
psql -h /tmp/pgrun -p 55432 -U postgres -c 'CREATE DATABASE doccheck;'
PSQL="psql -h /tmp/pgrun -p 55432 -U postgres -d doccheck -v ON_ERROR_STOP=1"
$PSQL -f migrations/001_initial_schema.sql && $PSQL -f migrations/002_bootstrap_seed.sql
$PSQL -f /tmp/doc_sql.sql        # the blocks copied out of the doc
$PSQL -c '\d public.<table>'     # then assert the resulting shape
```

Assert outcomes, not just exit codes: check the post-state of pre-existing rows, and try to insert the duplicate/violating row you claim is prevented.

### 3. Render every mermaid block

```bash
npm install @mermaid-js/mermaid-cli && npx mmdc -i block.mmd -o block.svg
```

Use ASCII subgraph ids with quoted CJK labels; a CJK id referenced by a later `style` line is a renderer-compatibility risk.

## DDL traps to check explicitly

| Trap | Check |
| --- | --- |
| `ADD COLUMN` with `DEFAULT` | Is that default semantically right for **existing** rows? If not, add an explicit `UPDATE` backfill. |
| `UNIQUE` over a nullable column | NULLs are never equal in Postgres; add a partial unique index for the NULL case. |
| Table written as `CREATE TABLE` | Confirm it does not already exist in an earlier migration; if it does, write `ALTER`. |
| Foreign key added | Both sides' type and width must match, or the FK will not build. |
| Column type narrowed/converted | Existing seed rows must be convertible, and their **content format** must still fit the new model. |

## Verification

- Every "current state" sentence in the doc maps to a command output you ran.
- Every SQL block executed on top of the repo's real migrations, with the resulting state asserted.
- Every mermaid block rendered.
- Fixes found this way go into the doc itself, and the verification log is attached to the PR.
