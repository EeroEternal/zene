const fs = require('fs');
const path = require('path');

const ROOT_DIR = path.resolve(__dirname, '..');
const SITE_DIR = __dirname;
const DIST_DIR = path.join(SITE_DIR, 'dist');

function ensureDir(dir) {
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }
}

function escapeHtml(str) {
  return str
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

function formatInlineMarkdown(text) {
  let result = escapeHtml(text);
  // Inline code: `code`
  result = result.replace(/`([^`]+)`/g, '<code>$1</code>');
  // Bold: **text**
  result = result.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  // Markdown links: [title](url)
  result = result.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');
  return result;
}

function parseChangelog(content) {
  const lines = content.split('\n');
  const releases = [];
  let currentRelease = null;
  let currentSection = null;
  let currentList = null;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    // Check version header: ## v0.1.16 (2026-09-07)
    const releaseMatch = line.match(/^##\s+(v[0-9.]+)\s*(?:\(([^)]+)\))?/);
    if (releaseMatch) {
      if (currentRelease) {
        releases.push(currentRelease);
      }
      currentRelease = {
        version: releaseMatch[1],
        date: releaseMatch[2] || '',
        sections: []
      };
      currentSection = null;
      currentList = null;
      continue;
    }

    if (!currentRelease) continue;

    // Check subheadings: ### Added, ### Changed, etc.
    const sectionMatch = line.match(/^###\s+(.+)$/);
    if (sectionMatch) {
      currentSection = {
        title: sectionMatch[1].trim(),
        items: []
      };
      currentRelease.sections.push(currentSection);
      currentList = currentSection.items;
      continue;
    }

    // Check bullet points
    const bulletMatch = line.match(/^(\s*)-\s+(.+)$/);
    if (bulletMatch && currentList) {
      const indent = bulletMatch[1].length;
      const text = bulletMatch[2].trim();
      currentList.push({ indent, text });
      continue;
    }

    // Continuation of a bullet item
    if (line.trim().length > 0 && currentList && currentList.length > 0) {
      const last = currentList[currentList.length - 1];
      last.text += ' ' + line.trim();
    }
  }

  if (currentRelease) {
    releases.push(currentRelease);
  }

  return releases;
}

function renderReleasesHtml(releases) {
  return releases.map((rel, index) => {
    let sectionsHtml = '';
    const sectionBadges = rel.sections.map(s => {
      const cls = s.title.toLowerCase().replace(/[^a-z0-9]/g, '');
      return `<span class="section-pill pill-${cls}">${escapeHtml(s.title)} (${s.items.length})</span>`;
    }).join(' ');

    for (const sec of rel.sections) {
      let itemsHtml = sec.items.map(item => {
        return `<li>${formatInlineMarkdown(item.text)}</li>`;
      }).join('\n');

      sectionsHtml += `
        <div class="changelog-group">
          <h3>${escapeHtml(sec.title)}</h3>
          <ul>${itemsHtml}</ul>
        </div>
      `;
    }

    const isOpen = index === 0 ? ' open' : '';

    return `
      <details class="release-card" id="${rel.version}"${isOpen}>
        <summary class="release-summary">
          <div class="release-summary-left">
            <span class="release-version">${escapeHtml(rel.version)}</span>
            <span class="release-date">${escapeHtml(rel.date)}</span>
            <span class="release-badges">${sectionBadges}</span>
          </div>
          <div class="release-summary-right">
            <span class="chevron-icon">›</span>
          </div>
        </summary>
        <div class="changelog-content">
          ${sectionsHtml}
        </div>
      </details>
    `;
  }).join('\n');
}

function build() {
  console.log('Building zene.sh site...');
  ensureDir(DIST_DIR);

  // 1. Read Cargo.toml for current version
  const cargoContent = fs.readFileSync(path.join(ROOT_DIR, 'Cargo.toml'), 'utf-8');
  const versionMatch = cargoContent.match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/);
  const version = versionMatch ? versionMatch[1] : '0.1.16';
  console.log(`Detected version: v${version}`);

  // 2. Read and parse CHANGELOG.md
  const changelogContent = fs.readFileSync(path.join(ROOT_DIR, 'CHANGELOG.md'), 'utf-8');
  const releases = parseChangelog(changelogContent);
  console.log(`Parsed ${releases.length} releases from CHANGELOG.md`);
  const changelogHtml = renderReleasesHtml(releases);

  // 3. Render index.html from template
  const templateHtml = fs.readFileSync(path.join(SITE_DIR, 'template.html'), 'utf-8');
  const finalHtml = templateHtml
    .replace(/\{\{VERSION\}\}/g, version)
    .replace('<!-- CHANGELOG_INJECTION_POINT -->', changelogHtml);

  fs.writeFileSync(path.join(DIST_DIR, 'index.html'), finalHtml);
  console.log('Generated site/dist/index.html');

  // 4. Copy static assets
  fs.copyFileSync(path.join(SITE_DIR, 'style.css'), path.join(DIST_DIR, 'style.css'));
  fs.copyFileSync(path.join(SITE_DIR, 'llms.txt'), path.join(DIST_DIR, 'llms.txt'));
  fs.copyFileSync(path.join(SITE_DIR, 'llms-full.txt'), path.join(DIST_DIR, 'llms-full.txt'));
  fs.copyFileSync(path.join(SITE_DIR, '_headers'), path.join(DIST_DIR, '_headers'));
  fs.copyFileSync(path.join(SITE_DIR, '_redirects'), path.join(DIST_DIR, '_redirects'));

  // 5. Copy raw project docs and installer
  fs.copyFileSync(path.join(ROOT_DIR, 'CHANGELOG.md'), path.join(DIST_DIR, 'CHANGELOG.md'));
  fs.copyFileSync(path.join(ROOT_DIR, 'README.md'), path.join(DIST_DIR, 'README.md'));
  fs.copyFileSync(path.join(ROOT_DIR, 'scripts', 'install-release.sh'), path.join(DIST_DIR, 'install.sh'));

  console.log('Successfully staged all assets into site/dist/');
}

build();
