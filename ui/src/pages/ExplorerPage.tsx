import { useState, useEffect, useCallback } from 'react'
import { client, type GqlNode, type GqlEdge } from '@/lib/grpc-client'
import { useAppStore } from '@/lib/store'
import { cn } from '@/lib/utils'
import { NodeList } from '@/components/explorer/NodeList'
import { EdgeList } from '@/components/explorer/EdgeList'
import { NodeForm } from '@/components/explorer/NodeForm'

type NodeData = { id: string; labels: string[]; properties: Record<string, unknown> }
type EdgeData = { id: string; label: string; source: string; target: string; properties: Record<string, unknown> }
type Tab = 'nodes' | 'edges'

export function ExplorerPage() {
  const currentGraph = useAppStore((s) => s.currentGraph)
  const [tab, setTab] = useState<Tab>('nodes')
  const [showForm, setShowForm] = useState(false)
  const [nodes, setNodes] = useState<NodeData[]>([])
  const [edges, setEdges] = useState<EdgeData[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string>()

  const fetchNodes = useCallback(() => {
    return client.executeQuery('MATCH (n) RETURN n LIMIT 200', currentGraph)
      .then((res) => {
        if (res.error) { setError(res.error); return }
        const parsed: NodeData[] = []
        for (const row of res.rows) {
          const val = row[res.columns[0]] as GqlNode | null
          if (val && typeof val === 'object' && 'id' in val && 'labels' in val) {
            parsed.push({ id: val.id, labels: val.labels, properties: val.properties ?? {} })
          }
        }
        setNodes(parsed)
      })
  }, [currentGraph])

  const fetchEdges = useCallback(() => {
    return client.executeQuery('MATCH ()-[r]->() RETURN r LIMIT 200', currentGraph)
      .then((res) => {
        if (res.error) { setError(res.error); return }
        const parsed: EdgeData[] = []
        for (const row of res.rows) {
          const val = row[res.columns[0]] as GqlEdge | null
          if (val && typeof val === 'object' && 'id' in val && 'label' in val) {
            parsed.push({
              id: val.id, label: val.label,
              source: val.sourceId, target: val.targetId,
              properties: val.properties ?? {},
            })
          }
        }
        setEdges(parsed)
      })
  }, [currentGraph])

  useEffect(() => {
    setLoading(true)
    setError(undefined)
    Promise.all([fetchNodes(), fetchEdges()])
      .catch((err) => setError(err instanceof Error ? err.message : 'Failed to load data'))
      .finally(() => setLoading(false))
  }, [fetchNodes, fetchEdges])

  const nodeLabels = Array.from(new Set(nodes.flatMap((n) => n.labels)))
  const edgeLabels = Array.from(new Set(edges.map((e) => e.label)))

  return (
    <div className="flex h-full flex-col gap-4">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Data Explorer</h1>
        <button
          onClick={() => setShowForm(!showForm)}
          className="rounded bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground hover:bg-primary/90"
        >
          {tab === 'nodes' ? 'Create Node' : 'Create Edge'}
        </button>
      </div>

      {error && (
        <div className="rounded-lg border border-red-500/30 bg-red-500/10 p-3 text-sm text-red-400">
          {error}
        </div>
      )}

      <div className="flex gap-1 border-b border-border">
        {(['nodes', 'edges'] as const).map((t) => (
          <button
            key={t}
            onClick={() => {
              setTab(t)
              setShowForm(false)
            }}
            className={cn(
              'px-4 py-2 text-sm font-medium capitalize transition-colors',
              tab === t
                ? 'border-b-2 border-primary text-foreground'
                : 'text-muted-foreground hover:text-foreground'
            )}
          >
            {t}
          </button>
        ))}
      </div>

      {showForm && tab === 'nodes' && (
        <NodeForm
          onSave={(data) => {
            const labels = data.labels.join(':')
            const props = data.properties
              .filter((p) => p.key)
              .map((p) => `${p.key}: '${p.value}'`)
              .join(', ')
            const gql = `INSERT (:${labels} {${props}})`
            client.executeQuery(gql, currentGraph).then(() => {
              setShowForm(false)
              fetchNodes()
            })
          }}
          onCancel={() => setShowForm(false)}
        />
      )}

      {showForm && tab === 'edges' && (
        <NodeForm
          initialLabels={[]}
          onSave={(data) => {
            console.log('Create edge:', data)
            setShowForm(false)
          }}
          onCancel={() => setShowForm(false)}
        />
      )}

      {loading ? (
        <div className="flex items-center gap-2 py-12 justify-center text-muted-foreground">
          <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-current border-t-transparent" />
          Loading…
        </div>
      ) : (
        <div className="rounded-lg border border-border bg-card p-4">
          {tab === 'nodes' && (
            nodes.length === 0
              ? <p className="text-muted-foreground text-sm py-4 text-center">No nodes found. Run INSERT queries or seed demo data from the Schema page.</p>
              : <NodeList nodes={nodes} availableLabels={nodeLabels} />
          )}
          {tab === 'edges' && (
            edges.length === 0
              ? <p className="text-muted-foreground text-sm py-4 text-center">No edges found.</p>
              : <EdgeList edges={edges} availableLabels={edgeLabels} />
          )}
        </div>
      )}
    </div>
  )
}
