import { useState, useCallback } from 'react'
import { client, type QueryResult } from '@/lib/grpc-client'
import { useAppStore } from '@/lib/store'
import { QueryEditor } from '@/components/query/QueryEditor'
import { ResultsTable } from '@/components/query/ResultsTable'
import { QueryStats } from '@/components/query/QueryStats'
import { QueryHistory } from '@/components/query/QueryHistory'

export function QueryPage() {
  const [query, setQuery] = useState("MATCH (n) RETURN n.name, n.age, n.city LIMIT 10")
  const [results, setResults] = useState<QueryResult | null>(null)
  const [isLoading, setIsLoading] = useState(false)
  const [error, setError] = useState<string | undefined>()
  const [historyOpen, setHistoryOpen] = useState(true)
  const addQueryHistory = useAppStore((s) => s.addQueryHistory)
  const currentGraph = useAppStore((s) => s.currentGraph)

  const execute = useCallback(async (q: string) => {
    const trimmed = q.trim()
    if (!trimmed) return
    setIsLoading(true)
    setError(undefined)
    const start = Date.now()
    try {
      const result = await client.executeQuery(trimmed, currentGraph)
      const elapsed = Date.now() - start
      setResults(result)
      setError(result.error)
      addQueryHistory({
        id: crypto.randomUUID(),
        query: trimmed,
        timestamp: Date.now(),
        executionTimeMs: result.stats?.executionTimeMs ?? elapsed,
        rowCount: result.rows.length,
        error: result.error,
      })
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Unknown error'
      setError(msg)
    } finally {
      setIsLoading(false)
    }
  }, [currentGraph, addQueryHistory])

  const handleSelectHistory = useCallback((q: string) => {
    setQuery(q)
  }, [])

  return (
    <div className="flex h-full overflow-hidden">
      {/* History sidebar */}
      {historyOpen && (
        <div className="w-64 shrink-0 border-r border-border bg-card">
          <QueryHistory onSelect={handleSelectHistory} />
        </div>
      )}

      {/* Main area */}
      <div className="flex flex-1 flex-col overflow-hidden">
        {/* Toolbar */}
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <button
            onClick={() => setHistoryOpen((o) => !o)}
            className="rounded px-2 py-1 text-xs text-muted-foreground hover:bg-secondary hover:text-foreground"
            title="Toggle history"
          >
            ☰
          </button>
          <button
            onClick={() => execute(query)}
            disabled={isLoading}
            className="inline-flex items-center gap-1.5 rounded bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:opacity-50"
          >
            {isLoading ? (
              <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-t-transparent" />
            ) : (
              <span>▶</span>
            )}
            Execute
          </button>
          <span className="text-xs text-muted-foreground">Ctrl+Enter</span>
        </div>

        {/* Editor */}
        <div className="h-1/2 min-h-[150px] shrink-0 overflow-hidden">
          <QueryEditor value={query} onChange={setQuery} onExecute={execute} />
        </div>

        {/* Results */}
        <div className="flex flex-1 flex-col overflow-auto border-t border-border">
          {results?.stats && <QueryStats stats={results.stats} />}
          <div className="flex-1 overflow-auto p-2">
            <ResultsTable
              columns={results?.columns ?? []}
              rows={results?.rows ?? []}
              error={error}
            />
          </div>
        </div>
      </div>
    </div>
  )
}
