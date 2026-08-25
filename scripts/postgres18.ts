import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { dirname, join } from 'node:path'

export interface Postgres18Tools {
  initdb: string
  pgCtl: string
  pgBasebackup: string
}

function brewPostgres18Bin(): string | undefined {
  if (process.platform !== 'darwin') return undefined
  try {
    const prefix = execFileSync('brew', ['--prefix', 'postgresql@18'], { encoding: 'utf8' }).trim()
    return prefix ? join(prefix, 'bin') : undefined
  } catch {
    return undefined
  }
}

function candidateBinDirs(): string[] {
  const configured = process.env.ELECTRIC_CIRCUITS_PG_BIN_DIR ?? process.env.PG_BIN_DIR
  return [
    configured,
    brewPostgres18Bin(),
    '/opt/homebrew/opt/postgresql@18/bin',
    '/usr/local/opt/postgresql@18/bin',
    '/usr/lib/postgresql/18/bin',
  ].filter((dir): dir is string => Boolean(dir))
}

function findBin(name: string): string {
  for (const dir of candidateBinDirs()) {
    const path = join(dir, name)
    if (existsSync(path)) return path
  }

  try {
    const path = execFileSync('which', [name], { encoding: 'utf8' }).trim()
    if (path) return path
  } catch {
    // handled by the actionable error below
  }
  throw new Error(
    `PostgreSQL 18 is required, but ${name} was not found. ` +
      'Install PostgreSQL 18 or set ELECTRIC_CIRCUITS_PG_BIN_DIR to its bin directory.',
  )
}

function versionOf(path: string): string {
  return execFileSync(path, ['--version'], { encoding: 'utf8' }).trim()
}

export function postgres18Tools(): Postgres18Tools {
  const initdb = findBin('initdb')
  const version = versionOf(initdb)
  if (!/^initdb \(PostgreSQL\) 18\./.test(version)) {
    throw new Error(
      `PostgreSQL 18 is required for the ephemeral database, but ${version || 'unknown version'} was selected from ${dirname(initdb)}. ` +
        'Install PostgreSQL 18, update PATH, or set ELECTRIC_CIRCUITS_PG_BIN_DIR.',
    )
  }
  const binDir = dirname(initdb)
  const pgCtl = join(binDir, 'pg_ctl')
  const pgBasebackup = join(binDir, 'pg_basebackup')
  if (!existsSync(pgCtl)) throw new Error(`PostgreSQL 18 pg_ctl is missing beside ${initdb}`)
  if (!existsSync(pgBasebackup)) throw new Error(`PostgreSQL 18 pg_basebackup is missing beside ${initdb}`)
  return { initdb, pgCtl, pgBasebackup }
}
