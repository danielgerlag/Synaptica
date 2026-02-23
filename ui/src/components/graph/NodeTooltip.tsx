import type { GraphNode, GraphEdge } from './types'

const COLORS = [
  '#3b82f6', '#ef4444', '#22c55e', '#f59e0b', '#8b5cf6', '#ec4899',
  '#06b6d4', '#f97316', '#14b8a6', '#6366f1', '#84cc16', '#e11d48',
]

function labelColor(label: string): string {
  let h = 0
  for (let i = 0; i < label.length; i++) h = label.charCodeAt(i) + ((h << 5) - h)
  return COLORS[Math.abs(h) % COLORS.length]
}

interface NodeTooltipProps {
  node?: GraphNode
  edge?: GraphEdge
  onClose: () => void
}

export function NodeTooltip({ node, edge, onClose }: NodeTooltipProps) {
  if (!node && !edge) return null

  return (
    <div className="absolute right-0 top-0 z-10 h-full w-80 border-l border-border bg-card p-4 shadow-lg transition-transform">
      <div className="mb-4 flex items-center justify-between">
        <h3 className="text-sm font-semibold text-foreground">
          {node ? 'Node Details' : 'Edge Details'}
        </h3>
        <button
          onClick={onClose}
          className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M18 6L6 18M6 6l12 12" />
          </svg>
        </button>
      </div>

      {node && (
        <div className="space-y-3">
          <div>
            <span className="text-xs text-muted-foreground">ID</span>
            <p className="text-sm font-mono text-foreground">{node.id}</p>
          </div>
          <div>
            <span className="text-xs text-muted-foreground">Labels</span>
            <div className="mt-1 flex flex-wrap gap-1">
              {node.labels.map((label) => (
                <span
                  key={label}
                  className="rounded-full px-2 py-0.5 text-xs font-medium text-white"
                  style={{ backgroundColor: labelColor(label) }}
                >
                  {label}
                </span>
              ))}
            </div>
          </div>
          <div>
            <span className="text-xs text-muted-foreground">Properties</span>
            <table className="mt-1 w-full text-sm">
              <tbody>
                {Object.entries(node.properties).map(([key, value]) => (
                  <tr key={key} className="border-t border-border">
                    <td className="py-1 pr-2 font-medium text-muted-foreground">{key}</td>
                    <td className="py-1 text-foreground">{String(value)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {edge && (
        <div className="space-y-3">
          <div>
            <span className="text-xs text-muted-foreground">ID</span>
            <p className="text-sm font-mono text-foreground">{edge.id}</p>
          </div>
          <div>
            <span className="text-xs text-muted-foreground">Label</span>
            <p className="text-sm font-semibold text-foreground">{edge.label}</p>
          </div>
          <div>
            <span className="text-xs text-muted-foreground">Source</span>
            <p className="text-sm font-mono text-foreground">
              {typeof edge.source === 'string' ? edge.source : edge.source.id}
            </p>
          </div>
          <div>
            <span className="text-xs text-muted-foreground">Target</span>
            <p className="text-sm font-mono text-foreground">
              {typeof edge.target === 'string' ? edge.target : edge.target.id}
            </p>
          </div>
          <div>
            <span className="text-xs text-muted-foreground">Properties</span>
            <table className="mt-1 w-full text-sm">
              <tbody>
                {Object.entries(edge.properties).map(([key, value]) => (
                  <tr key={key} className="border-t border-border">
                    <td className="py-1 pr-2 font-medium text-muted-foreground">{key}</td>
                    <td className="py-1 text-foreground">{String(value)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  )
}
