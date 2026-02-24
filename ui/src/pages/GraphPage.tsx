import { useState, useEffect, useMemo, useCallback } from 'react'
import { client, type GqlNode, type GqlEdge } from '@/lib/grpc-client'
import { useAppStore } from '@/lib/store'
import { GraphCanvas } from '@/components/graph/GraphCanvas'
import { GraphControls } from '@/components/graph/GraphControls'
import { NodeTooltip } from '@/components/graph/NodeTooltip'
import type { GraphData, GraphNode, GraphEdge } from '@/components/graph/types'

export function GraphPage() {
  const currentGraph = useAppStore((s) => s.currentGraph)
  const [graphData, setGraphData] = useState<GraphData>({ nodes: [], edges: [] })
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string>()

  const [selectedNode, setSelectedNode] = useState<GraphNode | undefined>()
  const [selectedEdge, setSelectedEdge] = useState<GraphEdge | undefined>()
  const [visibleLabels, setVisibleLabels] = useState<Set<string>>(new Set())
  const [layoutKey, setLayoutKey] = useState(0)

  useEffect(() => {
    setLoading(true)
    setError(undefined)
    client.executeQuery('MATCH (n)-[r]->(m) RETURN n, r, m LIMIT 200', currentGraph)
      .then((res) => {
        if (res.error) { setError(res.error); return }
        const nodeMap = new Map<string, GraphNode>()
        const edgeMap = new Map<string, GraphEdge>()

        for (const row of res.rows) {
          const n = row['n'] as GqlNode | null
          const r = row['r'] as GqlEdge | null
          const m = row['m'] as GqlNode | null

          if (n && typeof n === 'object' && 'id' in n) {
            nodeMap.set(n.id, { id: n.id, labels: n.labels ?? [], properties: n.properties ?? {} })
          }
          if (m && typeof m === 'object' && 'id' in m) {
            nodeMap.set(m.id, { id: m.id, labels: m.labels ?? [], properties: m.properties ?? {} })
          }
          if (r && typeof r === 'object' && 'id' in r) {
            edgeMap.set(r.id, {
              id: r.id, label: r.label ?? '',
              source: r.sourceId, target: r.targetId,
              properties: r.properties ?? {},
            })
          }
        }

        const data: GraphData = {
          nodes: Array.from(nodeMap.values()),
          edges: Array.from(edgeMap.values()),
        }
        setGraphData(data)

        const labels = new Set<string>()
        data.nodes.forEach((n) => n.labels.forEach((l) => labels.add(l)))
        setVisibleLabels(labels)
      })
      .catch((err) => setError(err instanceof Error ? err.message : 'Failed to load graph'))
      .finally(() => setLoading(false))
  }, [currentGraph])

  const allLabels = useMemo(() => {
    const s = new Set<string>()
    graphData.nodes.forEach((n) => n.labels.forEach((l) => s.add(l)))
    return Array.from(s)
  }, [graphData])

  const filteredData = useMemo<GraphData>(() => {
    const nodes = graphData.nodes.filter((n) => n.labels.some((l) => visibleLabels.has(l)))
    const nodeIds = new Set(nodes.map((n) => n.id))
    const edges = graphData.edges.filter((e) => {
      const sid = typeof e.source === 'string' ? e.source : e.source.id
      const tid = typeof e.target === 'string' ? e.target : e.target.id
      return nodeIds.has(sid) && nodeIds.has(tid)
    })
    return { nodes, edges }
  }, [graphData, visibleLabels])

  const handleToggleLabel = useCallback((label: string) => {
    setVisibleLabels((prev) => {
      const next = new Set(prev)
      if (next.has(label)) next.delete(label)
      else next.add(label)
      return next
    })
  }, [])

  const handleNodeClick = useCallback((node: GraphNode) => {
    setSelectedNode(node)
    setSelectedEdge(undefined)
  }, [])

  const handleEdgeClick = useCallback((edge: GraphEdge) => {
    setSelectedEdge(edge)
    setSelectedNode(undefined)
  }, [])

  const handleCloseTooltip = useCallback(() => {
    setSelectedNode(undefined)
    setSelectedEdge(undefined)
  }, [])

  const handleFitToScreen = useCallback(() => {
    setLayoutKey((k) => k + 1)
  }, [])

  const handleResetLayout = useCallback(() => {
    setLayoutKey((k) => k + 1)
    handleCloseTooltip()
  }, [handleCloseTooltip])

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground">
        <span className="inline-block h-4 w-4 animate-spin rounded-full border-2 border-current border-t-transparent mr-2" />
        Loading graph…
      </div>
    )
  }

  if (error) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2">
        <div className="rounded-lg border border-red-500/30 bg-red-500/10 p-3 text-sm text-red-400">{error}</div>
      </div>
    )
  }

  if (graphData.nodes.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-muted-foreground">
        No graph data to visualize. Run a query or seed demo data from the Schema page.
      </div>
    )
  }

  return (
    <div className="flex h-full flex-col gap-2">
      <GraphControls
        labels={allLabels}
        visibleLabels={visibleLabels}
        onToggleLabel={handleToggleLabel}
        onFitToScreen={handleFitToScreen}
        onResetLayout={handleResetLayout}
        nodeCount={filteredData.nodes.length}
        edgeCount={filteredData.edges.length}
      />
      <div className="relative flex-1 overflow-hidden rounded-lg border border-border">
        <GraphCanvas
          key={layoutKey}
          data={filteredData}
          onNodeClick={handleNodeClick}
          onEdgeClick={handleEdgeClick}
        />
        <NodeTooltip node={selectedNode} edge={selectedEdge} onClose={handleCloseTooltip} />
      </div>
    </div>
  )
}
