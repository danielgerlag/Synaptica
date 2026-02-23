import { useAppStore } from '@/lib/store'

interface QueryHistoryProps {
  onSelect: (query: string) => void
}

function timeAgo(ts: number): string {
  const seconds = Math.floor((Date.now() - ts) / 1000)
  if (seconds < 60) return 'just now'
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  return `${days}d ago`
}

export function QueryHistory({ onSelect }: QueryHistoryProps) {
  const queryHistory = useAppStore((s) => s.queryHistory)
  const clearQueryHistory = useAppStore((s) => s.clearQueryHistory)

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-sm font-medium">History</span>
        {queryHistory.length > 0 && (
          <button
            onClick={clearQueryHistory}
            className="text-xs text-muted-foreground hover:text-foreground"
          >
            Clear
          </button>
        )}
      </div>
      <div className="flex-1 overflow-y-auto">
        {queryHistory.length === 0 ? (
          <p className="p-3 text-xs text-muted-foreground">No queries yet</p>
        ) : (
          queryHistory.map((entry) => (
            <button
              key={entry.id}
              onClick={() => onSelect(entry.query)}
              className="block w-full border-b border-border px-3 py-2 text-left hover:bg-secondary/50"
            >
              <p className="truncate text-xs font-mono text-foreground">
                {entry.query}
              </p>
              <div className="mt-0.5 flex items-center gap-2 text-xs text-muted-foreground">
                <span>{timeAgo(entry.timestamp)}</span>
                {entry.executionTimeMs != null && (
                  <span>{entry.executionTimeMs}ms</span>
                )}
                {entry.error && (
                  <span className="text-red-400">error</span>
                )}
              </div>
            </button>
          ))
        )}
      </div>
    </div>
  )
}
