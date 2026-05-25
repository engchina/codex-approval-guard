import { readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import process from 'node:process'
import { pathToFileURL } from 'node:url'
import { normalizeVersion, validateRelease } from './validate-release.mjs'
import { extractChangelogSection } from './build-release-body.mjs'

const PACKAGE_JSON_PATH = 'package.json'
const TAURI_CONFIG_PATH = 'src-tauri/tauri.conf.json'
const CARGO_TOML_PATH = 'src-tauri/Cargo.toml'
const CHANGELOG_PATH = 'CHANGELOG.md'
const DEFAULT_UNRELEASED_SCAFFOLD = [
  {
    heading: '### Added',
    hint: '新しいガード対象、自動承認/閉鎖ルール、UI 改善など、ユーザーが気付く新機能。',
  },
  {
    heading: '### Changed',
    hint: '既存挙動・既定値・ワークフローの変更で、ユーザーが日常的に気付くもの。',
  },
  {
    heading: '### Fixed',
    hint: '自動承認/閉鎖の精度、誤検出、UI、安定性などの不具合修正。',
  },
  {
    heading: '### Internal',
    hint: 'リファクタ、ツール、テスト、リリースプロセスなど、プロジェクト履歴に残したい保守作業。',
  },
]

export function normalizeReleaseDate(value = new Date()) {
  if (value instanceof Date) {
    const year = value.getFullYear()
    const month = String(value.getMonth() + 1).padStart(2, '0')
    const day = String(value.getDate()).padStart(2, '0')
    return `${year}-${month}-${day}`
  }

  const normalized = String(value ?? '').trim()
  if (/^\d{4}-\d{2}-\d{2}$/u.test(normalized)) {
    return normalized
  }

  throw new Error(`Expected a release date like "2026-05-01", received "${normalized || '(empty)'}".`)
}

export function replaceJsonVersion(source, version) {
  const eol = detectEol(source)
  const data = JSON.parse(source)
  data.version = normalizeVersion(version)
  const nextSource = `${JSON.stringify(data, null, 2)}\n`
  return eol === '\n' ? nextSource : nextSource.replace(/\n/gu, eol)
}

export function replaceCargoPackageVersion(source, version) {
  const nextVersion = normalizeVersion(version)
  const eol = detectEol(source)
  const lines = source.split(/\r?\n/u)
  let inPackageSection = false
  let replaced = false

  for (let index = 0; index < lines.length; index += 1) {
    const trimmed = lines[index].trim()

    if (trimmed === '[package]') {
      inPackageSection = true
      continue
    }

    if (inPackageSection && /^\[.+\]$/u.test(trimmed)) {
      break
    }

    if (!inPackageSection) {
      continue
    }

    if (/^version\s*=\s*"[^"]+"/u.test(trimmed)) {
      lines[index] = lines[index].replace(/version\s*=\s*"[^"]+"/u, `version = "${nextVersion}"`)
      replaced = true
      break
    }
  }

  if (!replaced) {
    throw new Error('Unable to update version in src-tauri/Cargo.toml.')
  }

  const nextSource = lines.join(eol)
  return source.endsWith(eol) ? `${nextSource}${eol}` : nextSource
}

export function extractUnreleasedChangelogSection(source) {
  const lines = source.split(/\r?\n/u)
  const startIndex = lines.findIndex((line) => line.trim() === '## Unreleased')
  if (startIndex === -1) return null

  const bodyLines = []
  for (let index = startIndex + 1; index < lines.length; index += 1) {
    const line = lines[index]
    if (/^##\s+/u.test(line)) break
    bodyLines.push(line)
  }

  const section = bodyLines.join('\n').trim()
  return section.length > 0 ? section : null
}

export function isUnreleasedScaffoldHintLine(line) {
  const trimmed = line.trim()
  return /^<!--.*-->$/u.test(trimmed)
}

export function hasMeaningfulUnreleasedContent(section) {
  return section
    .split(/\r?\n/u)
    .some((line) => {
      const trimmed = line.trim()
      return trimmed.length > 0 && !/^###\s+/u.test(trimmed) && !isUnreleasedScaffoldHintLine(trimmed)
    })
}

export function stripUnreleasedScaffoldHints(section) {
  return section
    .split(/\r?\n/u)
    .filter((line) => !isUnreleasedScaffoldHintLine(line))
    .join('\n')
    .replace(/\n{3,}/gu, '\n\n')
    .trim()
}

export function buildSuggestedUnreleasedScaffold() {
  return DEFAULT_UNRELEASED_SCAFFOLD
    .map(({ heading, hint }) => [heading, '', `<!-- ${hint} -->`].join('\n'))
    .join('\n\n')
}

export function finalizeUnreleasedChangelog({
  source,
  version,
  date = new Date(),
}) {
  const nextVersion = normalizeVersion(version)
  const releaseDate = normalizeReleaseDate(date)
  const eol = detectEol(source)
  const unreleasedSection = extractUnreleasedChangelogSection(source)
  const releasedSection = unreleasedSection ? stripUnreleasedScaffoldHints(unreleasedSection) : null

  if (!unreleasedSection) {
    throw new Error('CHANGELOG.md is missing a populated "## Unreleased" section.')
  }

  if (!hasMeaningfulUnreleasedContent(unreleasedSection)) {
    throw new Error('CHANGELOG ## Unreleased does not contain any release notes to promote.')
  }

  if (extractChangelogSection(source, nextVersion)) {
    throw new Error(`CHANGELOG.md already contains a v${nextVersion} section.`)
  }

  const lines = source.split(/\r?\n/u)
  const startIndex = lines.findIndex((line) => line.trim() === '## Unreleased')
  let endIndex = lines.length
  for (let index = startIndex + 1; index < lines.length; index += 1) {
    if (/^##\s+/u.test(lines[index])) {
      endIndex = index
      break
    }
  }

  const before = lines.slice(0, startIndex).join('\n').trimEnd()
  const after = lines.slice(endIndex).join('\n').trimStart()
  const unreleasedHeading = ['## Unreleased', '', buildSuggestedUnreleasedScaffold()].join('\n').trim()
  const releaseHeading = [`## ${nextVersion} - ${releaseDate}`, '', releasedSection].join('\n').trim()

  const nextSource = [before, unreleasedHeading, releaseHeading, after]
    .filter((part) => part.length > 0)
    .join('\n\n')
    .trimEnd() + '\n'

  return eol === '\n' ? nextSource : nextSource.replace(/\n/gu, eol)
}

export async function prepareRelease({
  version,
  date = new Date(),
  cwd = process.cwd(),
} = {}) {
  const nextVersion = normalizeVersion(version)
  if (!nextVersion) {
    throw new Error('Expected a version argument such as "0.2.0" or "v0.2.0".')
  }

  const releaseDate = normalizeReleaseDate(date)
  const [packageSource, tauriSource, cargoSource, changelogSource] = await Promise.all([
    readFile(path.join(cwd, PACKAGE_JSON_PATH), 'utf8'),
    readFile(path.join(cwd, TAURI_CONFIG_PATH), 'utf8'),
    readFile(path.join(cwd, CARGO_TOML_PATH), 'utf8'),
    readFile(path.join(cwd, CHANGELOG_PATH), 'utf8'),
  ])

  const previousVersion = normalizeVersion(JSON.parse(packageSource).version)
  const nextPackageSource = replaceJsonVersion(packageSource, nextVersion)
  const nextTauriSource = replaceJsonVersion(tauriSource, nextVersion)
  const nextCargoSource = replaceCargoPackageVersion(cargoSource, nextVersion)
  const nextChangelogSource = finalizeUnreleasedChangelog({
    source: changelogSource,
    version: nextVersion,
    date: releaseDate,
  })

  await Promise.all([
    writeFile(path.join(cwd, PACKAGE_JSON_PATH), nextPackageSource, 'utf8'),
    writeFile(path.join(cwd, TAURI_CONFIG_PATH), nextTauriSource, 'utf8'),
    writeFile(path.join(cwd, CARGO_TOML_PATH), nextCargoSource, 'utf8'),
    writeFile(path.join(cwd, CHANGELOG_PATH), nextChangelogSource, 'utf8'),
  ])

  await validateRelease({
    version: nextVersion,
    cwd,
    requireChangelogSection: true,
  })

  return {
    version: nextVersion,
    previousVersion,
    date: releaseDate,
  }
}

export function formatPreparedReleaseSummary({
  version,
  previousVersion,
  date,
}) {
  return [
    `Prepared release v${version}.`,
    `- Previous app version: ${previousVersion || '(unknown)'}`,
    `- package.json -> ${version}`,
    `- src-tauri/tauri.conf.json -> ${version}`,
    `- src-tauri/Cargo.toml -> ${version}`,
    `- CHANGELOG.md: promoted ## Unreleased to ## ${version} - ${date}`,
    '- CHANGELOG.md: seeded the next `## Unreleased` section with a suggested scaffold.',
    `Next: review CHANGELOG.md and docs/release-notes-draft.md, then run "npm run release:validate -- ${version}" and "npm run release:notes:preview -- ${version}".`,
  ].join('\n')
}

function detectEol(source) {
  return source.includes('\r\n') ? '\r\n' : '\n'
}

function isDirectExecution() {
  if (!process.argv[1]) return false
  return import.meta.url === pathToFileURL(process.argv[1]).href
}

if (isDirectExecution()) {
  const args = process.argv.slice(2)
  const rawVersion = args.find((value) => !value.startsWith('--')) ?? ''
  const dateIndex = args.findIndex((value) => value === '--date')
  const rawDate = dateIndex >= 0 ? args[dateIndex + 1] : undefined

  try {
    const prepared = await prepareRelease({
      version: rawVersion,
      date: rawDate,
    })
    process.stdout.write(`${formatPreparedReleaseSummary(prepared)}\n`)
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  }
}
