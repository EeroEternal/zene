# zene.sh — Static Documentation & Agent Portal

This directory contains the static documentation portal for `zene.sh`, deployed on **Cloudflare Pages**.

## Cloudflare Pages Configuration

- **Framework preset**: `None`
- **Build command**: `bash site/build.sh`
- **Build output directory**: `site/dist`
- **Root directory**: `/` (repository root)
- **Environment variables**: None required

## Features for Agents & Developers
- Pre-rendered semantic HTML with zero client-side framework overhead.
- Dynamic extraction and formatting of root `CHANGELOG.md` into release cards.
- AI Agent Discovery standard: `/llms.txt` and `/llms-full.txt`.
- Raw markdown and script endpoints: `/CHANGELOG.md`, `/README.md`, `/install.sh`.
- Fast Cloudflare Edge caching configured via `_headers` and `_redirects`.

## Local Testing
```bash
# Build the site into site/dist
bash site/build.sh

# Preview locally (e.g. using Python's built-in HTTP server or npx serve)
cd site/dist && python3 -m http.server 8080
```
