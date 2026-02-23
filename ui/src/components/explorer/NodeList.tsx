import { useState, useMemo } from 'react'

interface NodeData {
  id: string
  labels: string[]
  properties: Record<string, unknown>
}

interface NodeListProps {
  nodes: NodeData[]
  availableLabels: string[]
}

const PAGE_SIZE = 25

export function NodeList({ nodes, availableLabels }: NodeListProps) {
  const [selectedLabel, setSelectedLabel] = useState<string>('All')
  const [page, setPage] = useState(0)

  const filtered = useMemo(
    () =>
      selectedLabel === 'All'
        ? nodes
        : nodes.filter((n) => n.labels.includes(selectedLabel)),
    [nodes, selectedLabel]
  )

  const propColumns = useMemo(() => {
    const keys = new Set<string>()
    filtered.forEach((n) => Object.keys(n.properties).forEach((k) => keys.add(k)))
    return Array.from(keys)
  }, [filtered])

  const totalPages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE))
  const pageNodes = filtered.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE)
  const start = page * PAGE_SIZE + 1
  const end = Math.min((page + 1) * PAGE_SIZE, filtered.length)

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <label className="text-xs text-muted-foreground">Label:</label>
        <select
          value={selectedLabel}
          onChange={(e) => {
            setSelectedLabel(e.target.value)
            setPage(0)
          }}
          className="rounded border border-border bg-background px-2.5 py-1.5 text-sm text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
        >
          <option value="All">All</option>
          {availableLabels.map((l) => (
            <option key={l} value={l}>
              {l}
            </option>
          ))}
        </select>
      </div>

      <div className="overflow-auto rounded-lg border border-border">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border bg-secondary/50">
              <th className="px-3 py-2 text-left font-medium text-foreground">ID</th>
              <th className="px-3 py-2 text-left font-medium text-foreground">Labels</th>
              {propColumns.map((col) => (
                <th key={col} className="px-3 py-2 text-left font-medium text-foreground">
                  {col}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {pageNodes.length === 0 && (
              <tr>
                <td
                  colSpan={2 + propColumns.length}
                  className="px-3 py-4 text-center text-muted-foreground"
                >
                  No nodes found
                </td>
              </tr>
            )}
            {pageNodes.map((node) => (
              <tr key={node.id} className="border-b border-border last:border-0 hover:bg-secondary/30">
                <td className="px-3 py-2 font-mono text-xs">{node.id.slice(0, 8)}</td>
                <td className="px-3 py-2">
                  <div className="flex gap-1">
                    {node.labels.map((l) => (
                      <span
                        key={l}
                        className="rounded bg-primary/20 px-1.5 py-0.5 text-xs font-medium text-primary"
                      >
                        {l}
                      </span>
                    ))}
                  </div>
                </td>
                {propColumns.map((col) => (
                  <td key={col} className="px-3 py-2">
                    {node.properties[col] !== undefined ? String(node.properties[col]) : (
                      <span className="italic text-muted-foreground">—</span>
                    )}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {filtered.length > 0 && (
        <div className="flex items-center justify-between text-xs text-muted-foreground">
          <span>
            {start}–{end} of {filtered.length}
          </span>
          <div className="flex gap-2">
            <button
              onClick={() => setPage((p) => Math.max(0, p - 1))}
              disabled={page === 0}
              className="rounded border border-border px-2 py-1 hover:bg-secondary disabled:opacity-40"
            >
              Prev
            </button>
            <button
              onClick={() => setPage((p) => Math.min(totalPages - 1, p + 1))}
              disabled={page >= totalPages - 1}
              className="rounded border border-border px-2 py-1 hover:bg-secondary disabled:opacity-40"
            >
              Next
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
