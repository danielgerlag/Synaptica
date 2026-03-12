import { useState, useCallback, useRef, useEffect } from 'react'
import { client, type QueryResult } from '@/lib/grpc-client'
import { useAppStore } from '@/lib/store'
import { QueryEditor } from '@/components/query/QueryEditor'
import { ResultsTable } from '@/components/query/ResultsTable'
import { QueryStats } from '@/components/query/QueryStats'
import { QueryHistory } from '@/components/query/QueryHistory'

const QUERY_TEMPLATES = [
  { label: 'MATCH — find nodes', query: 'MATCH (n:Person)\nRETURN n.name, n.age\nLIMIT 10' },
  { label: 'MATCH — with filter', query: "MATCH (n:Person)\nWHERE n.age > 30\nRETURN n.name, n.age\nORDER BY n.age DESC" },
  { label: 'MATCH — traversal', query: "MATCH (a:Person)-[e:KNOWS]->(b:Person)\nRETURN a.name, b.name, TYPE(e)" },
  { label: 'MATCH — 2-hop path', query: "MATCH (a:Person)-[:KNOWS]->(b:Person)-[:KNOWS]->(c:Person)\nRETURN a.name, b.name, c.name" },
  { label: 'INSERT — node', query: "INSERT (:Person {name: 'Alice', age: 30, city: 'NYC'})" },
  { label: 'INSERT — edge', query: "MATCH (a:Person), (b:Person)\nWHERE a.name = 'Alice' AND b.name = 'Bob'\nINSERT (a)-[:KNOWS]->(b)" },
  { label: 'SET — update property', query: "MATCH (n:Person)\nWHERE n.name = 'Alice'\nSET n.age = 31\nRETURN n.name, n.age" },
  { label: 'DELETE — remove node', query: "MATCH (n:Person)\nWHERE n.name = 'Alice'\nDETACH DELETE n" },
  { label: 'Aggregate — COUNT', query: "MATCH (n:Person)\nRETURN COUNT(*) AS total" },
  { label: 'Aggregate — GROUP BY', query: "MATCH (n:Person)\nRETURN n.city AS city, COUNT(*) AS cnt\nORDER BY cnt DESC" },
  { label: 'WITH — pipeline', query: "MATCH (n:Person)\nWITH n.city AS city, COUNT(*) AS cnt\nWHERE cnt > 1\nRETURN city, cnt" },
  { label: 'CREATE INDEX', query: "CREATE INDEX idx_name FOR (n:Person) ON (n.name)" },
  { label: 'DROP INDEX', query: "DROP INDEX idx_name" },
  { label: 'CREATE GRAPH', query: "CREATE GRAPH myGraph" },
]

function TemplateDropdown({ onSelect }: { onSelect: (q: string) => void }) {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [open])

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen((o) => !o)}
        className="inline-flex items-center gap-1 rounded border border-border px-2.5 py-1.5 text-sm text-muted-foreground hover:bg-secondary hover:text-foreground"
        title="Query templates"
      >
        <span className="text-xs">📝</span>
        Templates
        <span className="text-[10px]">▼</span>
      </button>
      {open && (
        <div className="absolute left-0 top-full z-50 mt-1 w-64 rounded-md border border-border bg-card shadow-lg">
          <div className="max-h-80 overflow-y-auto py-1">
            {QUERY_TEMPLATES.map((t, i) => (
              <button
                key={i}
                onClick={() => { onSelect(t.query); setOpen(false) }}
                className="w-full px-3 py-2 text-left text-sm text-foreground hover:bg-secondary"
              >
                {t.label}
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  )
}

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
          <div className="mx-1 h-5 w-px bg-border" />
          <TemplateDropdown onSelect={(q) => setQuery(q)} />
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
