import { useState, useMemo, useCallback } from 'react'
import { GraphCanvas } from '@/components/graph/GraphCanvas'
import { GraphControls } from '@/components/graph/GraphControls'
import { NodeTooltip } from '@/components/graph/NodeTooltip'
import type { GraphData, GraphNode, GraphEdge } from '@/components/graph/types'

const DEMO_DATA: GraphData = {
  nodes: [
    { id: '1', labels: ['Person'], properties: { name: 'Alice', age: 30 } },
    { id: '2', labels: ['Person'], properties: { name: 'Bob', age: 25 } },
    { id: '3', labels: ['Person'], properties: { name: 'Charlie', age: 35 } },
    { id: '4', labels: ['Company'], properties: { name: 'Acme Corp', industry: 'Tech' } },
    { id: '5', labels: ['City'], properties: { name: 'New York', population: 8336817 } },
    { id: '6', labels: ['City'], properties: { name: 'San Francisco', population: 873965 } },
  ],
  edges: [
    { id: 'e1', source: '1', target: '2', label: 'KNOWS', properties: { since: 2020 } },
    { id: 'e2', source: '2', target: '3', label: 'KNOWS', properties: { since: 2019 } },
    { id: 'e3', source: '1', target: '3', label: 'KNOWS', properties: { since: 2021 } },
    { id: 'e4', source: '1', target: '4', label: 'WORKS_AT', properties: { role: 'Engineer' } },
    { id: 'e5', source: '2', target: '4', label: 'WORKS_AT', properties: { role: 'Designer' } },
    { id: 'e6', source: '1', target: '5', label: 'LIVES_IN', properties: {} },
    { id: 'e7', source: '2', target: '6', label: 'LIVES_IN', properties: {} },
    { id: 'e8', source: '3', target: '5', label: 'LIVES_IN', properties: {} },
  ],
}

export function GraphPage() {
  const [selectedNode, setSelectedNode] = useState<GraphNode | undefined>()
  const [selectedEdge, setSelectedEdge] = useState<GraphEdge | undefined>()
  const [visibleLabels, setVisibleLabels] = useState<Set<string>>(() => {
    const all = new Set<string>()
    DEMO_DATA.nodes.forEach((n) => n.labels.forEach((l) => all.add(l)))
    return all
  })
  const [layoutKey, setLayoutKey] = useState(0)

  const allLabels = useMemo(() => {
    const s = new Set<string>()
    DEMO_DATA.nodes.forEach((n) => n.labels.forEach((l) => s.add(l)))
    return Array.from(s)
  }, [])

  const filteredData = useMemo<GraphData>(() => {
    const nodes = DEMO_DATA.nodes.filter((n) => n.labels.some((l) => visibleLabels.has(l)))
    const nodeIds = new Set(nodes.map((n) => n.id))
    const edges = DEMO_DATA.edges.filter((e) => {
      const sid = typeof e.source === 'string' ? e.source : e.source.id
      const tid = typeof e.target === 'string' ? e.target : e.target.id
      return nodeIds.has(sid) && nodeIds.has(tid)
    })
    return { nodes, edges }
  }, [visibleLabels])

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
