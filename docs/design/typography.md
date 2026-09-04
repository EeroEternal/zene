# Typography

Typography is system-based and utilitarian. The admin UI should favor clarity and stability over personality fonts.

- Headlines use the sans stack with semibold or bold weight for page titles and primary section headers.
- Body text stays in the sans stack and should remain readable in dense forms, tables, and settings panels.
- Technical identifiers such as API keys, model slugs, request IDs, and command samples use the mono stack.
- Small metadata should reduce size, not contrast. `muted-foreground` carries secondary information.

Text hierarchy (visual spec v1.0). Build rank with **weight, color, and spacing** — do not add extra sizes.

| Role | Size / line | Weight | Use |
| --- | --- | --- | --- |
| Page title | 20 / 28 (`text-xl`) | 600 | `PageHeader` only |
| Section title | 16 / 24 | 600 | Card / module title |
| Table header | 14 / 20 | 500 | Column labels |
| Body | 14 / 20 | 400 | Table cells, prose |
| Sidebar menu | 14 / 20 | 400 / 500 | Nav items |
| Label | 12 / 20 | 500 | KPI labels, field labels |
| Secondary | 12 / 16–20 | 400 | Hints, timestamps |
| Metric | 20 | 600 | KPI numbers (`tabular-nums`) |

Weights in product UI: **400 / 500 / 600** only. Do not use 30px / 24px marketing headlines on admin pages.

- Do not pair titles with a default subtitle.
- Explanatory copy is body/secondary — in empty states, dialogs, or errors, **not** under a page or card title.
- Do not add a subtitle that repeats the title in different words (for example title「API 密钥」plus「仅显示属于该用户的密钥」).
- Keys, tokens, and code samples: `mono-sm`.
- Live metrics use tabular numbers to avoid layout jitter.

Long text handling is part of typography, not an afterthought:

- entity names and prose should use wrapping strategies such as `break-words` or `line-clamp-2`
- multi-line request/response bodies (logs, chat) use `whitespace-pre-wrap break-words [overflow-wrap:anywhere]` so Chinese and English both wrap cleanly; do not use `break-all` on prose (it splits CJK mid-phrase and long Latin tokens poorly)
- keep expand/collapse controls on their own line under the body, not inline at the end of the paragraph
- keys, IDs, and model slugs may use `break-all`
- avoid assuming a single line for Chinese or translated copy
