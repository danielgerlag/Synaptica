import { useRef, useEffect, useCallback } from 'react'
import * as d3 from 'd3'
import type { GraphData, GraphNode, GraphEdge } from './types'

const COLORS = [
  '#3b82f6', '#ef4444', '#22c55e', '#f59e0b', '#8b5cf6', '#ec4899',
  '#06b6d4', '#f97316', '#14b8a6', '#6366f1', '#84cc16', '#e11d48',
]

function labelColor(label: string): string {
  let h = 0
  for (let i = 0; i < label.length; i++) h = label.charCodeAt(i) + ((h << 5) - h)
  return COLORS[Math.abs(h) % COLORS.length]
}

interface GraphCanvasProps {
  data: GraphData
  onNodeClick?: (node: GraphNode) => void
  onEdgeClick?: (edge: GraphEdge) => void
}

export function GraphCanvas({ data, onNodeClick, onEdgeClick }: GraphCanvasProps) {
  const svgRef = useRef<SVGSVGElement>(null)
  const containerRef = useRef<HTMLDivElement>(null)
  const simulationRef = useRef<d3.Simulation<GraphNode, GraphEdge> | null>(null)

  const getNodeRadius = useCallback(
    (node: GraphNode) => {
      const count = data.edges.filter((e) => {
        const sid = typeof e.source === 'string' ? e.source : e.source.id
        const tid = typeof e.target === 'string' ? e.target : e.target.id
        return sid === node.id || tid === node.id
      }).length
      return Math.min(8 + count * 3, 20)
    },
    [data.edges],
  )

  useEffect(() => {
    const svg = svgRef.current
    const container = containerRef.current
    if (!svg || !container) return

    const rect = container.getBoundingClientRect()
    const w = rect.width || 800
    const h = rect.height || 600

    // Deep-copy so D3 mutations don't affect parent state
    const nodes: GraphNode[] = data.nodes.map((n) => ({ ...n }))
    const edges: GraphEdge[] = data.edges.map((e) => ({ ...e }))

    const sel = d3.select(svg)
    sel.selectAll('*').remove()
    sel.attr('width', w).attr('height', h)

    // Arrow marker
    sel
      .append('defs')
      .append('marker')
      .attr('id', 'arrowhead')
      .attr('viewBox', '0 -5 10 10')
      .attr('refX', 20)
      .attr('refY', 0)
      .attr('markerWidth', 6)
      .attr('markerHeight', 6)
      .attr('orient', 'auto')
      .append('path')
      .attr('d', 'M0,-5L10,0L0,5')
      .attr('fill', '#6b7280')

    const g = sel.append('g')

    // Zoom
    const zoom = d3.zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.1, 4])
      .on('zoom', (event) => {
        g.attr('transform', event.transform)
      })
    sel.call(zoom)

    // Edge lines
    const linkGroup = g
      .append('g')
      .selectAll<SVGLineElement, GraphEdge>('line')
      .data(edges)
      .join('line')
      .attr('stroke', '#6b7280')
      .attr('stroke-width', 1.5)
      .attr('marker-end', 'url(#arrowhead)')
      .style('cursor', 'pointer')
      .on('click', (_event, d) => onEdgeClick?.(d))

    // Edge labels
    const edgeLabelGroup = g
      .append('g')
      .selectAll<SVGTextElement, GraphEdge>('text')
      .data(edges)
      .join('text')
      .text((d) => d.label)
      .attr('font-size', 9)
      .attr('fill', '#9ca3af')
      .attr('text-anchor', 'middle')
      .attr('dy', -4)

    // Node circles
    const nodeGroup = g
      .append('g')
      .selectAll<SVGCircleElement, GraphNode>('circle')
      .data(nodes)
      .join('circle')
      .attr('r', (d) => getNodeRadius(d))
      .attr('fill', (d) => labelColor(d.labels[0] ?? 'default'))
      .attr('stroke', '#1f2937')
      .attr('stroke-width', 1.5)
      .style('cursor', 'pointer')
      .on('click', (_event, d) => onNodeClick?.(d))

    // Node labels
    const nodeLabelGroup = g
      .append('g')
      .selectAll<SVGTextElement, GraphNode>('text')
      .data(nodes)
      .join('text')
      .text((d) => {
        const name = d.properties.name
        return typeof name === 'string' ? name : d.id
      })
      .attr('font-size', 10)
      .attr('fill', '#ffffff')
      .attr('text-anchor', 'middle')
      .attr('dy', (d) => getNodeRadius(d) + 14)

    // Drag behaviour
    const drag = d3
      .drag<SVGCircleElement, GraphNode>()
      .on('start', (event, d) => {
        if (!event.active) simulation.alphaTarget(0.3).restart()
        d.fx = d.x
        d.fy = d.y
      })
      .on('drag', (event, d) => {
        d.fx = event.x
        d.fy = event.y
      })
      .on('end', (event, d) => {
        if (!event.active) simulation.alphaTarget(0)
        d.fx = null
        d.fy = null
      })
    nodeGroup.call(drag)

    // Force simulation
    const simulation = d3
      .forceSimulation<GraphNode>(nodes)
      .force(
        'link',
        d3
          .forceLink<GraphNode, GraphEdge>(edges)
          .id((d) => d.id)
          .distance(100),
      )
      .force('charge', d3.forceManyBody().strength(-300))
      .force('center', d3.forceCenter(w / 2, h / 2))
      .force('collide', d3.forceCollide(30))
      .on('tick', () => {
        linkGroup
          .attr('x1', (d) => (d.source as GraphNode).x ?? 0)
          .attr('y1', (d) => (d.source as GraphNode).y ?? 0)
          .attr('x2', (d) => (d.target as GraphNode).x ?? 0)
          .attr('y2', (d) => (d.target as GraphNode).y ?? 0)

        edgeLabelGroup
          .attr('x', (d) => (((d.source as GraphNode).x ?? 0) + ((d.target as GraphNode).x ?? 0)) / 2)
          .attr('y', (d) => (((d.source as GraphNode).y ?? 0) + ((d.target as GraphNode).y ?? 0)) / 2)

        nodeGroup.attr('cx', (d) => d.x ?? 0).attr('cy', (d) => d.y ?? 0)

        nodeLabelGroup.attr('x', (d) => d.x ?? 0).attr('y', (d) => d.y ?? 0)
      })

    simulationRef.current = simulation

    // Resize observer
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width, height } = entry.contentRect
        sel.attr('width', width).attr('height', height)
        simulation.force('center', d3.forceCenter(width / 2, height / 2))
        simulation.alpha(0.3).restart()
      }
    })
    observer.observe(container)

    return () => {
      simulation.stop()
      observer.disconnect()
      simulationRef.current = null
    }
  }, [data, onNodeClick, onEdgeClick, getNodeRadius])

  return (
    <div ref={containerRef} className="h-full w-full overflow-hidden">
      <svg ref={svgRef} className="h-full w-full" />
    </div>
  )
}
