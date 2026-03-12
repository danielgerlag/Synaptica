import { useEffect, useState, useRef } from 'react'
import { Database, ChevronDown, Plus } from 'lucide-react'
import { useAppStore } from '@/lib/store'
import { client } from '@/lib/grpc-client'
import type { GraphInfo } from '@/lib/grpc-client'

export function GraphSelector() {
  const { currentGraph, setCurrentGraph, isConnected } = useAppStore()
  const [graphs, setGraphs] = useState<GraphInfo[]>([])
  const [open, setOpen] = useState(false)
  const [creating, setCreating] = useState(false)
  const [newName, setNewName] = useState('')
  const dropdownRef = useRef<HTMLDivElement>(null)

  const fetchGraphs = () => {
    if (!isConnected) return
    client.listGraphs()
      .then(setGraphs)
      .catch(() => setGraphs([]))
  }

  useEffect(() => {
    fetchGraphs()
    const interval = setInterval(fetchGraphs, 15_000)
    return () => clearInterval(interval)
  }, [isConnected])

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setOpen(false)
        setCreating(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [])

  const handleCreate = async () => {
    const name = newName.trim()
    if (!name) return
    try {
      await client.executeQuery(`CREATE GRAPH ${name}`)
      setNewName('')
      setCreating(false)
      setCurrentGraph(name)
      fetchGraphs()
    } catch { /* ignore */ }
  }

  return (
    <div className="relative" ref={dropdownRef}>
      <button
        onClick={() => { setOpen(!open); if (!open) fetchGraphs() }}
        className="flex items-center gap-1.5 rounded-md border border-border bg-background px-2.5 py-1.5 text-sm hover:bg-accent transition-colors"
        title="Select graph"
      >
        <Database className="h-3.5 w-3.5 text-muted-foreground" />
        <span className="max-w-[120px] truncate">{currentGraph}</span>
        <ChevronDown className="h-3 w-3 text-muted-foreground" />
      </button>

      {open && (
        <div className="absolute left-0 top-full z-50 mt-1 min-w-[180px] rounded-md border border-border bg-card shadow-lg">
          <div className="max-h-48 overflow-y-auto py-1">
            {graphs.length === 0 && (
              <div className="px-3 py-2 text-xs text-muted-foreground">No graphs found</div>
            )}
            {graphs.map((g) => (
              <button
                key={g.id}
                onClick={() => { setCurrentGraph(g.name); setOpen(false) }}
                className={`flex w-full items-center gap-2 px-3 py-1.5 text-sm hover:bg-accent transition-colors ${
                  g.name === currentGraph ? 'bg-accent/50 font-medium' : ''
                }`}
              >
                <Database className="h-3 w-3 text-muted-foreground" />
                {g.name}
              </button>
            ))}
          </div>

          <div className="border-t border-border p-1">
            {creating ? (
              <div className="flex items-center gap-1 px-1">
                <input
                  autoFocus
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter') handleCreate(); if (e.key === 'Escape') setCreating(false) }}
                  placeholder="Graph name…"
                  className="flex-1 rounded border border-border bg-background px-2 py-1 text-sm outline-none focus:ring-1 focus:ring-ring"
                />
                <button
                  onClick={handleCreate}
                  className="rounded px-2 py-1 text-sm text-primary hover:bg-accent"
                >
                  OK
                </button>
              </div>
            ) : (
              <button
                onClick={() => setCreating(true)}
                className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-sm text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
              >
                <Plus className="h-3 w-3" />
                New graph
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
