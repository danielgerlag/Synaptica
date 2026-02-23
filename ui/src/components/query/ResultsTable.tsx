import { useState, useMemo } from 'react'
import { cn } from '@/lib/utils'

interface ResultsTableProps {
  columns: string[]
  rows: Record<string, unknown>[]
  error?: string
}

type SortDir = 'asc' | 'desc'

function renderCell(value: unknown) {
  if (value === null || value === undefined) {
    return <span className="italic text-muted-foreground">null</span>
  }
  if (typeof value === 'boolean') {
    return (
      <span
        className={cn(
          'inline-block rounded px-1.5 py-0.5 text-xs font-medium',
          value ? 'bg-green-900/50 text-green-400' : 'bg-red-900/50 text-red-400'
        )}
      >
        {String(value)}
      </span>
    )
  }
  if (typeof value === 'number') {
    return <span className="tabular-nums">{value}</span>
  }
  if (typeof value === 'object') {
    return <code className="text-xs">{JSON.stringify(value)}</code>
  }
  return String(value)
}

export function ResultsTable({ columns, rows, error }: ResultsTableProps) {
  const [sortCol, setSortCol] = useState<string | null>(null)
  const [sortDir, setSortDir] = useState<SortDir>('asc')

  const handleSort = (col: string) => {
    if (sortCol === col) {
      setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'))
    } else {
      setSortCol(col)
      setSortDir('asc')
    }
  }

  const sortedRows = useMemo(() => {
    if (!sortCol) return rows
    return [...rows].sort((a, b) => {
      const av = a[sortCol]
      const bv = b[sortCol]
      if (av == null && bv == null) return 0
      if (av == null) return 1
      if (bv == null) return -1
      if (av < bv) return sortDir === 'asc' ? -1 : 1
      if (av > bv) return sortDir === 'asc' ? 1 : -1
      return 0
    })
  }, [rows, sortCol, sortDir])

  if (error) {
    return (
      <div className="rounded-lg border border-red-800 bg-red-950/50 p-4 text-red-400">
        <span className="font-medium">Error: </span>
        {error}
      </div>
    )
  }

  if (columns.length === 0) {
    return (
      <div className="flex items-center justify-center p-8 text-muted-foreground">
        No results
      </div>
    )
  }

  return (
    <div className="overflow-auto rounded-lg border border-border">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border bg-secondary/50">
            {columns.map((col) => (
              <th
                key={col}
                onClick={() => handleSort(col)}
                className="cursor-pointer px-3 py-2 text-left font-medium text-foreground select-none hover:bg-secondary"
              >
                {col}
                {sortCol === col && (
                  <span className="ml-1 text-muted-foreground">
                    {sortDir === 'asc' ? '↑' : '↓'}
                  </span>
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sortedRows.map((row, i) => (
            <tr key={i} className="border-b border-border last:border-0 hover:bg-secondary/30">
              {columns.map((col) => (
                <td
                  key={col}
                  className={cn(
                    'px-3 py-2',
                    typeof row[col] === 'number' && 'text-right'
                  )}
                >
                  {renderCell(row[col])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
