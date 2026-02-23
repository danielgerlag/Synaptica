interface QueryStatsProps {
  stats?: {
    executionTimeMs: number
    rowsReturned: number
    nodesCreated: number
    nodesDeleted: number
    edgesCreated: number
    edgesDeleted: number
    propertiesSet: number
  }
}

interface StatBadgeProps {
  label: string
  value: number
}

function StatBadge({ label, value }: StatBadgeProps) {
  return (
    <span className="inline-flex items-center gap-1 rounded bg-secondary px-2 py-0.5 text-xs text-muted-foreground">
      <span className="font-medium text-foreground">{value}</span> {label}
    </span>
  )
}

export function QueryStats({ stats }: QueryStatsProps) {
  if (!stats) return null

  return (
    <div className="flex flex-wrap items-center gap-2 px-1 py-1.5 text-sm">
      <StatBadge label={`row${stats.rowsReturned !== 1 ? 's' : ''} in ${stats.executionTimeMs}ms`} value={stats.rowsReturned} />
      {stats.nodesCreated > 0 && <StatBadge label="nodes created" value={stats.nodesCreated} />}
      {stats.nodesDeleted > 0 && <StatBadge label="nodes deleted" value={stats.nodesDeleted} />}
      {stats.edgesCreated > 0 && <StatBadge label="edges created" value={stats.edgesCreated} />}
      {stats.edgesDeleted > 0 && <StatBadge label="edges deleted" value={stats.edgesDeleted} />}
      {stats.propertiesSet > 0 && <StatBadge label="properties set" value={stats.propertiesSet} />}
    </div>
  )
}
