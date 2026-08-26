/** Insert a bounded batch of issues into a running LinearLite Postgres database.
 *
 * The database identity sequence owns issue IDs. This keeps inserts safe when the browser and
 * this helper run at the same time; timestamps are monotonic within this batch so the new rows
 * are easy to spot in a live view.
 */

import { pathToFileURL } from 'node:url'
import pgpkg from 'pg'

import { PRIORITIES, STATUSES, type Priority, type Status } from './src/schema.js'

export const MAX_COUNT = 1_000

export interface SeedOptions {
  count: number
  titlePrefix: string
  projectId?: number
  status: Status
  priority: Priority
}

const HELP = `Usage: pnpm linearlite:seed -- --count <n> [options]

Insert 1-${MAX_COUNT} issues into the running LinearLite database.

Options:
  --count <n>             Number of issues to insert (required)
  --title-prefix <text>   Prefix for generated titles (default: Swift live task)
  --project <id>          Existing project id (default: first project with a member)
  --status <status>       backlog|todo|in_progress|done|canceled (default: todo)
  --priority <priority>   none|low|medium|high|urgent (default: medium)
  -h, --help              Show this help

The connection string is read from DATABASE_URL, or LINEARLITE_DATABASE_URL. The demo startup
prints its ephemeral URL as "postgres → ..."; copy it into DATABASE_URL without logging it.
`

function valueAfter(args: string[], index: number, option: string): string {
  const value = args[index + 1]
  if (value === undefined || value.startsWith('-')) throw new Error(`${option} requires a value`)
  return value
}

function parsePositiveInt(raw: string, option: string): number {
  if (!/^\d+$/.test(raw)) throw new Error(`${option} must be a positive integer`)
  const value = Number(raw)
  if (!Number.isSafeInteger(value) || value < 1) throw new Error(`${option} must be a positive integer`)
  return value
}

export function parseArgs(args: string[]): SeedOptions | 'help' {
  let count: number | undefined
  let titlePrefix = 'Swift live task'
  let projectId: number | undefined
  let status: Status = 'todo'
  let priority: Priority = 'medium'

  for (let i = 0; i < args.length; i++) {
    const arg = args[i]!
    if (arg === '--') continue
    const equal = arg.indexOf('=')
    const name = equal < 0 ? arg : arg.slice(0, equal)
    const inline = equal < 0 ? undefined : arg.slice(equal + 1)
    if (name === '-h' || name === '--help') return 'help'
    if (name === '--count') {
      const raw = inline ?? valueAfter(args, i++, '--count')
      count = parsePositiveInt(raw, '--count')
      if (count > MAX_COUNT) throw new Error(`--count must be at most ${MAX_COUNT}`)
    } else if (name === '--title-prefix') {
      const raw = inline ?? valueAfter(args, i++, '--title-prefix')
      if (raw.length === 0 || raw.length > 120) throw new Error('--title-prefix must be 1-120 characters')
      titlePrefix = raw
    } else if (name === '--project') {
      projectId = parsePositiveInt(inline ?? valueAfter(args, i++, '--project'), '--project')
    } else if (name === '--status') {
      const raw = inline ?? valueAfter(args, i++, '--status')
      if (!(STATUSES as readonly string[]).includes(raw)) throw new Error(`--status must be one of ${STATUSES.join(', ')}`)
      status = raw as Status
    } else if (name === '--priority') {
      const raw = inline ?? valueAfter(args, i++, '--priority')
      if (!(PRIORITIES as readonly string[]).includes(raw)) throw new Error(`--priority must be one of ${PRIORITIES.join(', ')}`)
      priority = raw as Priority
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }

  if (count === undefined) throw new Error('--count is required')
  return { count, titlePrefix, projectId, status, priority }
}

export function usage(): string {
  return HELP
}

type ProjectAndUser = { project_id: string | number; username: string }

export async function seedTasks(databaseUrl: string, options: SeedOptions): Promise<number[]> {
  const client = new pgpkg.Client({ connectionString: databaseUrl })
  await client.connect()
  try {
    await client.query('BEGIN')
    const project = await client.query<ProjectAndUser>(
      options.projectId === undefined
        ? `SELECT p.id AS project_id, u.name AS username
             FROM projects p
             JOIN project_members pm ON pm.project_id = p.id
             JOIN users u ON u.id = pm.user_id
            ORDER BY p.id, u.id
            LIMIT 1`
        : `SELECT p.id AS project_id, u.name AS username
             FROM projects p
             JOIN project_members pm ON pm.project_id = p.id
             JOIN users u ON u.id = pm.user_id
            WHERE p.id = $1
            ORDER BY u.id
            LIMIT 1`,
      options.projectId === undefined ? [] : [options.projectId],
    )
    if (project.rowCount !== 1) {
      throw new Error(
        options.projectId === undefined
          ? 'no project with a member exists; start LinearLite first'
          : `project ${options.projectId} does not exist or has no members`,
      )
    }

    const { project_id: projectId, username } = project.rows[0]!
    const now = Date.now()
    const values: unknown[] = []
    const tuples = Array.from({ length: options.count }, (_, index) => {
      const created = now + index
      const offset = values.length
      values.push(
        `${options.titlePrefix} #${index + 1}`,
        `Inserted by the LinearLite task seeder at ${new Date(created).toISOString()}`,
        options.status,
        options.priority,
        username,
        projectId,
        created,
        created,
        created,
      )
      return `($${offset + 1}, $${offset + 2}, $${offset + 3}, $${offset + 4}, $${offset + 5}, ` +
        `$${offset + 6}, $${offset + 7}, $${offset + 8}, $${offset + 9})`
    })
    const inserted = await client.query<{ id: string | number }>(
      `INSERT INTO issues (title, description, status, priority, username, project_id, created, modified, kanbanorder)
       VALUES ${tuples.join(', ')}
       RETURNING id`,
      values,
    )
    await client.query('COMMIT')
    return inserted.rows.map((row) => Number(row.id))
  } catch (error) {
    await client.query('ROLLBACK').catch(() => undefined)
    throw error
  } finally {
    await client.end()
  }
}

async function main(): Promise<void> {
  let options: SeedOptions | 'help'
  try {
    options = parseArgs(process.argv.slice(2))
  } catch (error) {
    console.error(`linearlite:seed: ${error instanceof Error ? error.message : String(error)}\n\n${HELP}`)
    process.exitCode = 2
    return
  }
  if (options === 'help') {
    console.log(HELP)
    return
  }
  const databaseUrl = process.env.DATABASE_URL ?? process.env.LINEARLITE_DATABASE_URL
  if (!databaseUrl) {
    console.error('linearlite:seed: set DATABASE_URL (or LINEARLITE_DATABASE_URL) to the running demo database')
    process.exitCode = 2
    return
  }
  try {
    const ids = await seedTasks(databaseUrl, options)
    console.log(`inserted ${ids.length} LinearLite task${ids.length === 1 ? '' : 's'} (ids ${ids.join(', ')})`)
  } catch (error) {
    console.error(`linearlite:seed: insert failed: ${error instanceof Error ? error.message : String(error)}`)
    process.exitCode = 1
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) void main()
